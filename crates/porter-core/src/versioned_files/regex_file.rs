use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use regex::Regex;
use semver::Version;

use super::VersionedFile;

/// Generic regex-driven versioned file. The pattern must contain a named
/// capture group `(?P<version>...)`. The matched text within that group is
/// replaced with the new version on write; everything else is preserved.
///
/// The pattern is applied to the entire file contents (not line-by-line),
/// so multiline matches are supported. If multiple matches exist, all are
/// rewritten in lockstep.
#[derive(Debug)]
pub struct RegexFile {
    path: PathBuf,
    re: Regex,
    raw_pattern: String,
}

impl RegexFile {
    pub fn new(path: PathBuf, pattern: &str) -> Result<Self> {
        let re =
            Regex::new(pattern).with_context(|| format!("invalid regex pattern: {pattern}"))?;
        if !re.capture_names().flatten().any(|name| name == "version") {
            bail!("regex pattern must contain a named capture group `(?P<version>...)`");
        }
        Ok(Self {
            path,
            re,
            raw_pattern: pattern.to_string(),
        })
    }

    pub fn pattern(&self) -> &str {
        &self.raw_pattern
    }
}

impl VersionedFile for RegexFile {
    fn path(&self) -> &Path {
        &self.path
    }

    fn read_version(&self) -> Result<Version> {
        let body = fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        let cap = self.re.captures(&body).with_context(|| {
            format!(
                "{} did not match regex {:?}",
                self.path.display(),
                self.raw_pattern
            )
        })?;
        let raw = cap.name("version").unwrap().as_str();
        // Strip a leading `v` if present so callers can pin against either
        // `v0.6.0` or `0.6.0` text in the file.
        let stripped = raw.strip_prefix('v').unwrap_or(raw);
        Version::parse(stripped)
            .with_context(|| format!("parsing version {raw:?} from {}", self.path.display()))
    }

    fn write_version(&self, version: &Version) -> Result<()> {
        let body = fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        let mut hit = false;
        let new_body = self.re.replace_all(&body, |caps: &regex::Captures<'_>| {
            hit = true;
            let full = caps.get(0).unwrap().as_str();
            let m = caps.name("version").unwrap();
            let raw = m.as_str();
            let prefix = if raw.starts_with('v') { "v" } else { "" };
            let replacement = format!("{prefix}{version}");
            // Splice the replacement back into the full match so we keep
            // any surrounding text inside the regex.
            let start = m.start() - caps.get(0).unwrap().start();
            let end = m.end() - caps.get(0).unwrap().start();
            let mut out = String::with_capacity(full.len() + replacement.len());
            out.push_str(&full[..start]);
            out.push_str(&replacement);
            out.push_str(&full[end..]);
            out
        });
        if !hit {
            bail!(
                "{} did not match regex {:?}",
                self.path.display(),
                self.raw_pattern
            );
        }
        fs::write(&self.path, new_body.as_ref())
            .with_context(|| format!("writing {}", self.path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use tempfile::TempDir;

    fn setup(body: &str, pat: &str) -> (TempDir, RegexFile) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("file.tf");
        fs::write(&path, body).unwrap();
        let f = RegexFile::new(path, pat).unwrap();
        (dir, f)
    }

    #[test]
    fn reads_version_from_terraform_literal() {
        let body = indoc! {r#"
            variable "platform_chart_revision" {
              type    = string
              default = "v0.5.2"
            }
        "#};
        let (_d, f) = setup(body, r#"default\s*=\s*"(?P<version>v[0-9.]+)""#);
        assert_eq!(f.read_version().unwrap(), Version::new(0, 5, 2));
    }

    #[test]
    fn writes_preserves_v_prefix() {
        let body = indoc! {r#"
            platform_chart_revision = "v0.5.2"
        "#};
        let (_d, f) = setup(
            body,
            r#"platform_chart_revision\s*=\s*"(?P<version>v[0-9.]+)""#,
        );
        f.write_version(&Version::new(0, 6, 0)).unwrap();
        let after = fs::read_to_string(f.path()).unwrap();
        assert!(after.contains(r#"platform_chart_revision = "v0.6.0""#));
    }

    #[test]
    fn writes_without_v_prefix_when_absent() {
        let body = "image_tag = \"0.5.2\"\n";
        let (_d, f) = setup(body, r#"image_tag\s*=\s*"(?P<version>[0-9.]+)""#);
        f.write_version(&Version::new(0, 6, 0)).unwrap();
        let after = fs::read_to_string(f.path()).unwrap();
        assert_eq!(after, "image_tag = \"0.6.0\"\n");
    }

    #[test]
    fn rewrites_all_occurrences() {
        let body = indoc! {r#"
            image_tag = "v0.5.2"
            sidecar_tag = "v0.5.2"
        "#};
        let (_d, f) = setup(body, r#"_tag\s*=\s*"(?P<version>v[0-9.]+)""#);
        f.write_version(&Version::new(0, 6, 0)).unwrap();
        let after = fs::read_to_string(f.path()).unwrap();
        assert!(after.contains(r#"image_tag = "v0.6.0""#));
        assert!(after.contains(r#"sidecar_tag = "v0.6.0""#));
    }

    #[test]
    fn pattern_without_named_group_errors() {
        let dir = TempDir::new().unwrap();
        let err = RegexFile::new(dir.path().join("x"), r#"version = "[0-9.]+""#)
            .unwrap_err()
            .to_string();
        assert!(err.contains("version"));
    }

    #[test]
    fn no_match_errors_on_write() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("x");
        fs::write(&p, "nothing here\n").unwrap();
        let f = RegexFile::new(p, r#"tag\s*=\s*"(?P<version>v[0-9.]+)""#).unwrap();
        let err = f.write_version(&Version::new(1, 0, 0)).unwrap_err();
        assert!(err.to_string().contains("did not match"));
    }
}
