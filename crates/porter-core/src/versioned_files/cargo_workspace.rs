use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use semver::Version;
use toml_edit::{DocumentMut, Item, Value};

use super::VersionedFile;

/// Cargo workspace `Cargo.toml`. Reads/writes `[workspace.package].version`,
/// which is the canonical inheritable version field for workspace members
/// declaring `version.workspace = true`.
#[derive(Debug)]
pub struct CargoWorkspaceFile {
    path: PathBuf,
}

impl CargoWorkspaceFile {
    #[must_use]
    pub const fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl VersionedFile for CargoWorkspaceFile {
    fn path(&self) -> &Path {
        &self.path
    }

    fn read_version(&self) -> Result<Version> {
        let body = fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        let doc: DocumentMut = body.parse().context("invalid Cargo.toml")?;
        let v = doc
            .get("workspace")
            .and_then(|w| w.get("package"))
            .and_then(|p| p.get("version"))
            .and_then(|n| n.as_str())
            .with_context(|| {
                format!(
                    "{} has no [workspace.package].version field",
                    self.path.display()
                )
            })?;
        Version::parse(v).with_context(|| format!("parsing version {v:?} from Cargo.toml"))
    }

    fn write_version(&self, version: &Version) -> Result<()> {
        let body = fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        let mut doc: DocumentMut = body.parse().context("invalid Cargo.toml")?;
        let pkg = doc
            .get_mut("workspace")
            .and_then(|w| w.get_mut("package"))
            .and_then(|p| p.as_table_like_mut());
        let Some(pkg) = pkg else {
            bail!("{} has no [workspace.package] table", self.path.display());
        };
        // Update the existing item in place so toml_edit preserves the
        // surrounding decor (comments and whitespace). Replacing the whole
        // item with `value(...)` would discard the prefix/suffix.
        let new_string = Value::from(version.to_string());
        match pkg.get_mut("version") {
            Some(Item::Value(existing)) => {
                let decor = existing.decor().clone();
                let mut replacement = new_string;
                *replacement.decor_mut() = decor;
                *existing = replacement;
            }
            _ => {
                pkg.insert("version", Item::Value(new_string));
            }
        }
        fs::write(&self.path, doc.to_string())
            .with_context(|| format!("writing {}", self.path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use tempfile::TempDir;

    fn write_and_read(body: &str) -> (TempDir, CargoWorkspaceFile) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Cargo.toml");
        fs::write(&path, body).unwrap();
        let f = CargoWorkspaceFile::new(path);
        (dir, f)
    }

    #[test]
    fn reads_workspace_version() {
        let body = indoc! {r#"
            [workspace]
            members = ["a"]

            [workspace.package]
            version = "0.5.2"
            edition = "2024"
        "#};
        let (_dir, f) = write_and_read(body);
        assert_eq!(f.read_version().unwrap(), Version::new(0, 5, 2));
    }

    #[test]
    fn writes_preserves_other_fields_and_comments() {
        let body = indoc! {r#"
            # top comment
            [workspace]
            members = ["a"]

            [workspace.package]
            # version comment
            version = "0.5.2"
            edition = "2024"
            license = "Apache-2.0"
        "#};
        let (_dir, f) = write_and_read(body);
        f.write_version(&Version::new(0, 6, 0)).unwrap();
        let after = fs::read_to_string(f.path()).unwrap();
        assert!(after.contains("# top comment"));
        assert!(after.contains("# version comment"));
        assert!(after.contains(r#"version = "0.6.0""#));
        assert!(after.contains(r#"edition = "2024""#));
        assert!(after.contains(r#"license = "Apache-2.0""#));
        assert_eq!(f.read_version().unwrap(), Version::new(0, 6, 0));
    }

    #[test]
    fn writes_preserves_comment_above_version_with_workspace_root_table() {
        // Regression: an input that begins with `[workspace]` then descends
        // into `[workspace.package]` was dropping the comment-line above
        // `version` in the smoke test, while a similar input without the
        // root `[workspace]` table preserved it.
        let body = indoc! {r#"
            [workspace]
            members = []

            [workspace.package]
            # managed by porter
            version = "0.5.2"
            edition = "2024"
        "#};
        let (_dir, f) = write_and_read(body);
        f.write_version(&Version::new(0, 5, 3)).unwrap();
        let after = fs::read_to_string(f.path()).unwrap();
        assert!(
            after.contains("# managed by porter"),
            "lost comment, file is:\n{after}"
        );
        assert!(after.contains(r#"version = "0.5.3""#));
    }

    #[test]
    fn read_errors_on_invalid_toml() {
        let (_dir, f) = write_and_read("[workspace\nversion = \"0.1.0\"\n");
        let err = f.read_version().unwrap_err().to_string();
        assert!(
            err.contains("invalid Cargo.toml") || err.to_ascii_lowercase().contains("toml"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn read_errors_on_non_string_version() {
        let body = indoc! {"
            [workspace.package]
            version = 1
        "};
        let (_dir, f) = write_and_read(body);
        let err = f.read_version().unwrap_err().to_string();
        assert!(
            err.contains("workspace.package"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn read_errors_on_workspace_typo() {
        let body = indoc! {r#"
            [package.workspace]
            version = "0.5.2"
        "#};
        let (_dir, f) = write_and_read(body);
        let err = f.read_version().unwrap_err().to_string();
        assert!(
            err.contains("workspace.package"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn missing_field_errors_clearly() {
        let body = indoc! {r#"
            [workspace]
            members = ["a"]
        "#};
        let (_dir, f) = write_and_read(body);
        let err = f.read_version().unwrap_err().to_string();
        assert!(err.contains("workspace.package"));
    }
}
