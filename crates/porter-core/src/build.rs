//! `porter build` — drive per-artifact build commands.
//!
//! For Phase B we implement the `cli-binary` kind end-to-end (cross-
//! compiles via plain `cargo build --target ...`, archives, and
//! checksums) since porter publishes itself that way. Other kinds
//! (`oci-image`, `helm-chart`, `npm-package`, `python-wheel`) are
//! orchestrated by the release workflow shelling out to the right
//! external tool — `porter build` for those is a thin "print the
//! command we'd run" placeholder until Phase D when we wrap them in
//! attestation.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use flate2::Compression;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};

/// Output of a `cli-binary` build.
#[derive(Debug, Clone)]
pub struct BuildArtifact {
    /// Final tarball path, e.g. `dist/porter-x86_64-unknown-linux-gnu.tar.gz`.
    pub tarball: PathBuf,
    /// SHA-256 of the tarball, hex-encoded.
    pub sha256: String,
}

/// Build a CLI binary for one (cargo package, target) pair, archive it
/// as `<dist>/<binary>-<target>.tar.gz`, and return its SHA-256.
///
/// The tarball contains exactly one file: the binary itself, named
/// `<binary>` (no `.exe` suffix added — Windows targets aren't supported
/// in the initial cut). The archive layout matches what `setup-porter`
/// expects.
pub fn build_cli_binary(opts: &BuildOpts) -> Result<BuildArtifact> {
    let BuildOpts {
        manifest_dir,
        package,
        binary,
        target,
        dist,
        cargo,
    } = opts;

    fs::create_dir_all(dist).with_context(|| format!("creating dist dir {}", dist.display()))?;

    // Cross-compile via plain `cargo build`. The reusable workflow runs
    // each target on a matching runner so no cross-toolchain is needed.
    // Local builds will fail unless the user has the target installed;
    // that's fine — the error from cargo is clear.
    let status = Command::new(cargo)
        .args(["build", "--release", "--locked", "--manifest-path"])
        .arg(manifest_dir.join("Cargo.toml"))
        .args(["--package", package, "--bin", binary, "--target", target])
        .status()
        .with_context(|| format!("running {} build", cargo))?;
    if !status.success() {
        bail!("cargo build failed for {target} (exit {status})");
    }

    let bin_path = manifest_dir
        .join("target")
        .join(target)
        .join("release")
        .join(binary);
    if !bin_path.is_file() {
        bail!(
            "expected binary at {} but it does not exist",
            bin_path.display()
        );
    }

    let tarball = dist.join(format!("{binary}-{target}.tar.gz"));
    write_tarball(&tarball, &bin_path, binary)?;

    let sha256 = sha256_hex_file(&tarball)?;
    Ok(BuildArtifact { tarball, sha256 })
}

/// Build options for [`build_cli_binary`].
#[derive(Debug, Clone)]
pub struct BuildOpts {
    /// Directory containing the `Cargo.toml` to build.
    pub manifest_dir: PathBuf,
    /// Cargo package name.
    pub package: String,
    /// Binary name produced by the package (the `[[bin]] name` value).
    pub binary: String,
    /// Rust target triple.
    pub target: String,
    /// Directory to drop the tarball and checksum file into.
    pub dist: PathBuf,
    /// `cargo` executable to invoke. Defaults to `"cargo"` in the CLI.
    pub cargo: String,
}

/// Append a checksum line for `path` to `checksums.txt` in the same
/// directory, using the BSD-style `<sha>  <basename>` format that
/// `sha256sum -c -` and `shasum -a 256 -c -` both accept.
pub fn append_checksum(dist: &Path, artifact: &BuildArtifact) -> Result<PathBuf> {
    let basename = artifact
        .tarball
        .file_name()
        .context("tarball has no filename")?
        .to_string_lossy()
        .into_owned();
    let path = dist.join("checksums.txt");
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    writeln!(f, "{}  {}", artifact.sha256, basename)
        .with_context(|| format!("writing to {}", path.display()))?;
    Ok(path)
}

fn write_tarball(out: &Path, bin: &Path, name_in_archive: &str) -> Result<()> {
    let f = fs::File::create(out).with_context(|| format!("creating tarball {}", out.display()))?;
    let gz = GzEncoder::new(f, Compression::default());
    let mut tar = tar::Builder::new(gz);
    // `append_path_with_name` preserves mode (incl. exec bit) on unix.
    tar.append_path_with_name(bin, name_in_archive)
        .with_context(|| format!("adding {} to tarball", bin.display()))?;
    tar.finish().context("finalizing tarball")?;
    Ok(())
}

fn sha256_hex_file(path: &Path) -> Result<String> {
    let mut f =
        fs::File::open(path).with_context(|| format!("opening {} for hashing", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn append_checksum_writes_bsd_format_line() {
        let dir = TempDir::new().unwrap();
        let tarball = dir.path().join("porter-x86_64-unknown-linux-gnu.tar.gz");
        fs::write(&tarball, b"contents").unwrap();
        let art = BuildArtifact {
            tarball: tarball.clone(),
            sha256: "abcd1234".into(),
        };
        let p = append_checksum(dir.path(), &art).unwrap();
        let body = fs::read_to_string(&p).unwrap();
        assert_eq!(body, "abcd1234  porter-x86_64-unknown-linux-gnu.tar.gz\n");

        // Appending again adds a second line, doesn't replace.
        let art2 = BuildArtifact {
            tarball: tarball.clone(),
            sha256: "ef567890".into(),
        };
        append_checksum(dir.path(), &art2).unwrap();
        let body = fs::read_to_string(&p).unwrap();
        let lines: Vec<_> = body.lines().collect();
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn sha256_hex_file_matches_known_vector() {
        // sha256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("x");
        fs::write(&p, b"hello").unwrap();
        let h = sha256_hex_file(&p).unwrap();
        assert_eq!(
            h,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn write_tarball_roundtrips_a_single_file() {
        use std::io::Read as _;
        let dir = TempDir::new().unwrap();
        let bin = dir.path().join("porter");
        fs::write(&bin, b"#!/bin/sh\necho hi\n").unwrap();
        let tarball = dir.path().join("out.tar.gz");
        write_tarball(&tarball, &bin, "porter").unwrap();

        // Stream through entries; collecting into a Vec first invalidates
        // the per-entry reader because `tar` advances the underlying
        // stream lazily.
        let f = fs::File::open(&tarball).unwrap();
        let gz = flate2::read::GzDecoder::new(f);
        let mut archive = tar::Archive::new(gz);
        let mut found = 0;
        for entry in archive.entries().unwrap() {
            let mut e = entry.unwrap();
            assert_eq!(e.path().unwrap().to_str().unwrap(), "porter");
            let mut body = Vec::new();
            e.read_to_end(&mut body).unwrap();
            assert_eq!(body, b"#!/bin/sh\necho hi\n");
            found += 1;
        }
        assert_eq!(found, 1);
    }
}
