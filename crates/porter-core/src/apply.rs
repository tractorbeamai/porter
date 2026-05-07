use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use semver::Version;

use crate::changelog::{prepend_section, render_section, today_utc};
use crate::changeset::ChangesetSet;
use crate::config::Config;
use crate::version::{NextVersion, compute_next_version};
use crate::versioned_files::load_versioned_file;

/// Result of running `porter version`. Lists the files that were rewritten,
/// the version chosen, and the changeset paths consumed.
#[derive(Debug, Clone)]
pub struct ApplyResult {
    pub next: NextVersion,
    pub rewritten_files: Vec<PathBuf>,
    pub changelog_path: PathBuf,
    pub consumed_changesets: Vec<PathBuf>,
}

/// Read each versioned file and assert they all carry the same version. The
/// "lowest of all the files" version is what we treat as the current
/// release line.
pub fn current_version(root: &Path, config: &Config) -> Result<Version> {
    if config.versioned_files.is_empty() {
        bail!("porter.toml has no [[versioned_files]] entries");
    }
    let mut versions = Vec::new();
    for spec in &config.versioned_files {
        let f = load_versioned_file(root, spec)?;
        let v = f
            .read_version()
            .with_context(|| format!("reading current version from {}", f.path().display()))?;
        versions.push((f.path().to_path_buf(), v));
    }
    // Default to the first file's version, but warn (via error) if any
    // disagree — drift between versioned files is exactly the bug porter
    // exists to prevent.
    let (_first_path, first) = &versions[0];
    for (p, v) in &versions[1..] {
        if v != first {
            bail!(
                "versioned files disagree on current version: {} reports {}, {} reports {}",
                versions[0].0.display(),
                first,
                p.display(),
                v
            );
        }
    }
    Ok(first.clone())
}

/// Compute the next version, write it to every versioned file, prepend the
/// rendered section to the changelog, and remove the consumed changeset
/// files. If `dry_run` is true, no filesystem mutation occurs.
pub fn apply_next_version(
    root: &Path,
    config: &Config,
    dry_run: bool,
) -> Result<Option<ApplyResult>> {
    let set = ChangesetSet::load_from_dir(&root.join(&config.changesets.directory))?;
    if set.is_empty() {
        return Ok(None);
    }
    let current = current_version(root, config)?;
    let Some(next) = compute_next_version(&current, &set)? else {
        return Ok(None);
    };

    let mut rewritten = Vec::with_capacity(config.versioned_files.len());
    for spec in &config.versioned_files {
        let f = load_versioned_file(root, spec)?;
        if !dry_run {
            f.write_version(&next.next)
                .with_context(|| format!("writing version to {}", f.path().display()))?;
        }
        rewritten.push(f.path().to_path_buf());
    }

    let changelog_path = root.join(&config.release.changelog);
    if !dry_run {
        let section = render_section(&next.next, &today_utc(), &set);
        prepend_section(&changelog_path, &section)?;
    }

    let consumed = set
        .changesets
        .iter()
        .map(|c| c.path.clone())
        .collect::<Vec<_>>();
    if !dry_run {
        for p in &consumed {
            fs::remove_file(p).with_context(|| format!("removing changeset {}", p.display()))?;
        }
    }

    Ok(Some(ApplyResult {
        next,
        rewritten_files: rewritten,
        changelog_path,
        consumed_changesets: consumed,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::{Bump, write_changeset};
    use indoc::indoc;
    use tempfile::TempDir;

    fn fixture() -> TempDir {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("porter.toml"),
            indoc! {r#"
                [changesets]
                directory = ".changeset"

                [[versioned_files]]
                type = "cargo-workspace"
                path = "Cargo.toml"

                [[versioned_files]]
                type = "helm-chart"
                path = "Chart.yaml"

                [[versioned_files]]
                type = "package-json"
                path = "package.json"

                [[versioned_files]]
                type = "regex"
                path = "main.tf"
                pattern = 'platform_chart_revision\s*=\s*"(?P<version>v[0-9.]+)"'
            "#},
        )
        .unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            indoc! {r#"
                [workspace]
                members = []

                [workspace.package]
                version = "0.1.0"
            "#},
        )
        .unwrap();
        fs::write(
            dir.path().join("Chart.yaml"),
            indoc! {r#"
                apiVersion: v2
                name: example
                version: 0.1.0
                appVersion: "0.1.0"
            "#},
        )
        .unwrap();
        fs::write(
            dir.path().join("package.json"),
            indoc! {r#"
                {
                  "name": "example",
                  "version": "0.1.0"
                }
            "#},
        )
        .unwrap();
        fs::write(
            dir.path().join("main.tf"),
            "platform_chart_revision = \"v0.1.0\"\n",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join(".changeset")).unwrap();
        dir
    }

    #[test]
    fn no_changesets_yields_none() {
        let dir = fixture();
        let body = fs::read_to_string(dir.path().join("porter.toml")).unwrap();
        let cfg = Config::from_toml(&body).unwrap();
        let r = apply_next_version(dir.path(), &cfg, false).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn applies_bump_across_all_files() {
        let dir = fixture();
        write_changeset(
            &dir.path().join(".changeset"),
            "feat-attest",
            Bump::Minor,
            "Add attest subcommand.",
        )
        .unwrap();
        let body = fs::read_to_string(dir.path().join("porter.toml")).unwrap();
        let cfg = Config::from_toml(&body).unwrap();
        let r = apply_next_version(dir.path(), &cfg, false)
            .unwrap()
            .unwrap();
        // 0.1.0 with a minor changeset, pre-1.0 → patch bump → 0.1.1
        assert_eq!(r.next.next.to_string(), "0.1.1");
        assert!(
            fs::read_to_string(dir.path().join("Cargo.toml"))
                .unwrap()
                .contains("0.1.1")
        );
        assert!(
            fs::read_to_string(dir.path().join("Chart.yaml"))
                .unwrap()
                .contains("version: 0.1.1")
        );
        assert!(
            fs::read_to_string(dir.path().join("package.json"))
                .unwrap()
                .contains(r#""version": "0.1.1""#)
        );
        assert!(
            fs::read_to_string(dir.path().join("main.tf"))
                .unwrap()
                .contains(r#""v0.1.1""#)
        );
        // changelog created
        let cl = fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap();
        assert!(cl.contains("0.1.1"));
        assert!(cl.contains("Add attest subcommand."));
        // changesets consumed
        let remaining = fs::read_dir(dir.path().join(".changeset")).unwrap().count();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn dry_run_does_not_mutate() {
        let dir = fixture();
        write_changeset(
            &dir.path().join(".changeset"),
            "feat",
            Bump::Major,
            "breaking",
        )
        .unwrap();
        let body = fs::read_to_string(dir.path().join("porter.toml")).unwrap();
        let cfg = Config::from_toml(&body).unwrap();
        let _ = apply_next_version(dir.path(), &cfg, true).unwrap().unwrap();
        // 0.1.0 unchanged
        assert!(
            fs::read_to_string(dir.path().join("Cargo.toml"))
                .unwrap()
                .contains("0.1.0")
        );
        assert!(!dir.path().join("CHANGELOG.md").exists());
        let remaining = fs::read_dir(dir.path().join(".changeset")).unwrap().count();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn drift_between_files_errors() {
        let dir = fixture();
        // Drift: bump only Cargo.toml.
        fs::write(
            dir.path().join("Cargo.toml"),
            indoc! {r#"
                [workspace]
                members = []

                [workspace.package]
                version = "0.2.0"
            "#},
        )
        .unwrap();
        let body = fs::read_to_string(dir.path().join("porter.toml")).unwrap();
        let cfg = Config::from_toml(&body).unwrap();
        let err = current_version(dir.path(), &cfg).unwrap_err().to_string();
        assert!(err.contains("disagree"));
    }
}
