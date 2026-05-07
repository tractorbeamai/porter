use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use regex::Regex;
use semver::Version;

use super::VersionedFile;

/// Helm `Chart.yaml` adapter.
///
/// Updates the top-level `version` and (optionally) `appVersion` keys. We
/// use a targeted regex rewrite rather than a full YAML round-trip because
/// YAML parsers reflow comments and quoting; charts are hand-edited and
/// tend to have meaningful comments and field ordering we want to leave
/// alone.
#[derive(Debug)]
pub struct HelmChartFile {
    path: PathBuf,
    update_app_version: bool,
}

impl HelmChartFile {
    #[must_use]
    pub const fn new(path: PathBuf, update_app_version: bool) -> Self {
        Self {
            path,
            update_app_version,
        }
    }
}

impl VersionedFile for HelmChartFile {
    fn path(&self) -> &Path {
        &self.path
    }

    fn read_version(&self) -> Result<Version> {
        let body = fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        let v = read_top_level_string(&body, "version")
            .with_context(|| format!("{} has no top-level `version:` key", self.path.display()))?;
        Version::parse(&v)
            .with_context(|| format!("parsing version {v:?} from {}", self.path.display()))
    }

    fn write_version(&self, version: &Version) -> Result<()> {
        let body = fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        let mut updated = replace_top_level_string(&body, "version", &version.to_string())
            .with_context(|| format!("rewriting `version:` in {}", self.path.display()))?;
        if self.update_app_version && top_level_key_present(&updated, "appVersion") {
            updated = replace_top_level_string(&updated, "appVersion", &version.to_string())
                .with_context(|| format!("rewriting `appVersion:` in {}", self.path.display()))?;
        }
        fs::write(&self.path, updated)
            .with_context(|| format!("writing {}", self.path.display()))?;
        Ok(())
    }
}

fn key_regex(key: &str) -> Regex {
    // Match a top-level YAML key: anchored at start of line with no
    // indentation (so we don't pick up nested `version:` inside a
    // `dependencies:` list, which is indented). `space` captures the
    // whitespace between value and any trailing comment so we can preserve
    // it verbatim on rewrite.
    let pat = format!(
        r#"(?m)^(?P<prefix>{key}\s*:[ \t]*)(?P<q>"|'|)(?P<value>[^"'\r\n#]*?)(?P<close>"|'|)(?P<space>[ \t]*)(?P<trailing>(?:#[^\r\n]*)?)$"#,
        key = regex::escape(key)
    );
    // `key` was just escaped by `regex::escape`, so the only variable
    // bytes in `pat` form a literal sequence; the rest is a static
    // pattern we ship in this file. Compilation can't fail.
    #[expect(
        clippy::expect_used,
        reason = "regex is built from a static template plus an escaped key; cannot fail"
    )]
    Regex::new(&pat).expect("static regex compiles")
}

fn read_top_level_string(body: &str, key: &str) -> Result<String> {
    let re = key_regex(key);
    let cap = re.captures(body).context("key not found")?;
    // The static regex always contains a `value` named group, so the
    // capture group is guaranteed to be present whenever `captures`
    // matched. Propagate as an error rather than `.unwrap()` to keep
    // the function panic-free.
    Ok(cap
        .name("value")
        .context("BUG: regex missing `value` named group")?
        .as_str()
        .trim()
        .to_owned())
}

fn top_level_key_present(body: &str, key: &str) -> bool {
    key_regex(key).is_match(body)
}

fn replace_top_level_string(body: &str, key: &str, new_value: &str) -> Result<String> {
    let re = key_regex(key);
    let mut hit = false;
    let out = re.replace(body, |caps: &regex::Captures<'_>| {
        hit = true;
        let prefix = &caps["prefix"];
        let q = &caps["q"];
        let close = &caps["close"];
        // Preserve quoting style and original spacing before any trailing
        // comment. Helm's Chart.yaml semver field is unambiguous quoted or
        // not; quoting is a stylistic choice we leave alone.
        let space = caps.name("space").map_or("", |m| m.as_str());
        let trailing = caps.name("trailing").map_or("", |m| m.as_str());
        format!("{prefix}{q}{new_value}{close}{space}{trailing}")
    });
    if !hit {
        bail!("key {key:?} not present");
    }
    Ok(out.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use tempfile::TempDir;

    fn setup(body: &str) -> (TempDir, HelmChartFile) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Chart.yaml");
        fs::write(&path, body).unwrap();
        let f = HelmChartFile::new(path, true);
        (dir, f)
    }

    #[test]
    fn reads_version() {
        let body = indoc! {r#"
            apiVersion: v2
            name: example
            version: 0.1.0
            appVersion: "0.1.0"
        "#};
        let (_d, f) = setup(body);
        assert_eq!(f.read_version().unwrap(), Version::new(0, 1, 0));
    }

    #[test]
    fn writes_both_version_fields_preserves_quoting() {
        let body = indoc! {r#"
            apiVersion: v2
            name: example
            description: A platform chart
            version: 0.1.0
            appVersion: "0.1.0"
            type: application
        "#};
        let (_d, f) = setup(body);
        f.write_version(&Version::new(0, 6, 0)).unwrap();
        let after = fs::read_to_string(f.path()).unwrap();
        assert!(after.contains("version: 0.6.0"));
        assert!(after.contains(r#"appVersion: "0.6.0""#));
        assert!(after.contains("description: A platform chart"));
        assert!(after.contains("type: application"));
    }

    #[test]
    fn writes_version_only_when_update_app_version_false() {
        let body = indoc! {r#"
            apiVersion: v2
            name: example
            version: 0.1.0
            appVersion: "0.1.0"
        "#};
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Chart.yaml");
        fs::write(&path, body).unwrap();
        let f = HelmChartFile::new(path.clone(), false);
        f.write_version(&Version::new(1, 0, 0)).unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("version: 1.0.0"));
        assert!(after.contains(r#"appVersion: "0.1.0""#));
    }

    #[test]
    fn preserves_inline_comments() {
        let body = indoc! {r#"
            apiVersion: v2
            name: example
            version: 0.1.0  # release-managed
            appVersion: "0.1.0"  # release-managed
        "#};
        let (_d, f) = setup(body);
        f.write_version(&Version::new(0, 2, 0)).unwrap();
        let after = fs::read_to_string(f.path()).unwrap();
        assert!(after.contains("version: 0.2.0  # release-managed"));
        assert!(after.contains(r#"appVersion: "0.2.0"  # release-managed"#));
    }

    #[test]
    fn does_not_match_nested_version_key() {
        // Ensure we don't rewrite e.g. `dependencies[].version`.
        let body = indoc! {r#"
            apiVersion: v2
            name: example
            version: 0.1.0
            appVersion: "0.1.0"
            dependencies:
              - name: cnpg
                version: 0.21.0
                repository: https://cloudnative-pg.github.io/charts
        "#};
        let (_d, f) = setup(body);
        f.write_version(&Version::new(0, 2, 0)).unwrap();
        let after = fs::read_to_string(f.path()).unwrap();
        assert!(after.contains("version: 0.2.0"));
        assert!(after.contains("version: 0.21.0"));
    }
}
