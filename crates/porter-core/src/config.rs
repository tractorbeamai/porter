use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    #[serde(default)]
    pub changesets: ChangesetsConfig,
    #[serde(rename = "versioned_files", default)]
    pub versioned_files: Vec<VersionedFileSpec>,
    #[serde(rename = "artifacts", default)]
    pub artifacts: Vec<ArtifactConfig>,
    #[serde(default)]
    pub signing: Option<SigningConfig>,
    #[serde(default)]
    pub attestation: Option<AttestationConfig>,
    #[serde(default)]
    pub release: ReleaseConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangesetsConfig {
    #[serde(default = "default_changesets_dir")]
    pub directory: PathBuf,
    #[serde(default)]
    pub mode: ChangesetMode,
}

impl Default for ChangesetsConfig {
    fn default() -> Self {
        Self {
            directory: default_changesets_dir(),
            mode: ChangesetMode::default(),
        }
    }
}

fn default_changesets_dir() -> PathBuf {
    PathBuf::from(".changeset")
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ChangesetMode {
    /// All packages move together at the same version.
    #[default]
    Single,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum VersionedFileSpec {
    CargoWorkspace {
        path: PathBuf,
    },
    HelmChart {
        path: PathBuf,
        #[serde(default = "default_true")]
        update_app_version: bool,
    },
    PackageJson {
        path: PathBuf,
    },
    Regex {
        path: PathBuf,
        pattern: String,
    },
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ArtifactConfig {
    OciImage {
        name: String,
        context: PathBuf,
        dockerfile: PathBuf,
        registry: String,
        #[serde(default = "default_platforms")]
        platforms: Vec<String>,
    },
    HelmChart {
        name: String,
        chart: PathBuf,
        registry: String,
    },
    NpmPackage {
        name: String,
        path: PathBuf,
        #[serde(default = "default_npm_registry")]
        registry: String,
    },
    PythonWheel {
        name: String,
        path: PathBuf,
    },
    CliBinary {
        name: String,
        package: String,
        #[serde(default = "default_cli_targets")]
        targets: Vec<String>,
    },
}

fn default_platforms() -> Vec<String> {
    vec!["linux/amd64".into(), "linux/arm64".into()]
}

fn default_npm_registry() -> String {
    "https://registry.npmjs.org".into()
}

fn default_cli_targets() -> Vec<String> {
    vec![
        "x86_64-unknown-linux-gnu".into(),
        "aarch64-unknown-linux-gnu".into(),
        "x86_64-apple-darwin".into(),
        "aarch64-apple-darwin".into(),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SigningConfig {
    #[serde(default)]
    pub backend: SigningBackend,
    #[serde(default = "default_fulcio")]
    pub fulcio_url: String,
    #[serde(default = "default_rekor")]
    pub rekor_url: String,
}

fn default_fulcio() -> String {
    "https://fulcio.sigstore.dev".into()
}

fn default_rekor() -> String {
    "https://rekor.sigstore.dev".into()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SigningBackend {
    #[default]
    Sigstore,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttestationConfig {
    pub layout: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseConfig {
    #[serde(default = "default_tag_prefix")]
    pub tag_prefix: String,
    #[serde(default = "default_changelog_path")]
    pub changelog: PathBuf,
}

impl Default for ReleaseConfig {
    fn default() -> Self {
        Self {
            tag_prefix: default_tag_prefix(),
            changelog: default_changelog_path(),
        }
    }
}

fn default_tag_prefix() -> String {
    "v".into()
}

fn default_changelog_path() -> PathBuf {
    PathBuf::from("CHANGELOG.md")
}

impl Config {
    pub const FILENAME: &'static str = "porter.toml";

    pub fn load(path: &Path) -> Result<Self> {
        let body = fs::read_to_string(path)
            .with_context(|| format!("reading porter config {}", path.display()))?;
        Self::from_toml(&body).with_context(|| format!("parsing porter config {}", path.display()))
    }

    pub fn from_toml(body: &str) -> Result<Self> {
        toml::from_str(body).context("invalid porter.toml")
    }

    /// Find `porter.toml` by walking up from `start`.
    pub fn discover(start: &Path) -> Option<PathBuf> {
        let mut cur: Option<&Path> = Some(start);
        while let Some(dir) = cur {
            let candidate = dir.join(Self::FILENAME);
            if candidate.is_file() {
                return Some(candidate);
            }
            cur = dir.parent();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;

    #[test]
    fn parses_minimal_config() {
        let body = indoc! {r#"
            [[versioned_files]]
            type = "cargo-workspace"
            path = "Cargo.toml"
        "#};
        let cfg = Config::from_toml(body).unwrap();
        assert_eq!(cfg.versioned_files.len(), 1);
        assert!(matches!(
            cfg.versioned_files[0],
            VersionedFileSpec::CargoWorkspace { .. }
        ));
        assert_eq!(cfg.changesets.directory, PathBuf::from(".changeset"));
        assert_eq!(cfg.release.tag_prefix, "v");
    }

    #[test]
    fn parses_full_config() {
        let body = indoc! {r#"
            [changesets]
            directory = ".changes"
            mode = "single"

            [[versioned_files]]
            type = "cargo-workspace"
            path = "rust/Cargo.toml"

            [[versioned_files]]
            type = "helm-chart"
            path = "deploy/helm/foo/Chart.yaml"

            [[versioned_files]]
            type = "package-json"
            path = "ts/packages/sdk/package.json"

            [[versioned_files]]
            type = "regex"
            path = "deploy/main.tf"
            pattern = 'platform_chart_revision\s*=\s*"(?P<version>v[0-9.]+)"'

            [[artifacts]]
            kind = "oci-image"
            name = "api"
            context = "rust/"
            dockerfile = "rust/bins/api/Dockerfile"
            registry = "ghcr.io/example/api"

            [[artifacts]]
            kind = "helm-chart"
            name = "platform"
            chart = "deploy/helm/foo"
            registry = "oci://ghcr.io/example/charts"

            [signing]
            backend = "sigstore"

            [attestation]
            layout = "layouts/main.json"

            [release]
            tag_prefix = "v"
            changelog = "CHANGELOG.md"
        "#};
        let cfg = Config::from_toml(body).unwrap();
        assert_eq!(cfg.versioned_files.len(), 4);
        assert_eq!(cfg.artifacts.len(), 2);
        assert!(cfg.signing.is_some());
        assert!(cfg.attestation.is_some());
    }

    #[test]
    fn discovers_walks_up() {
        let tmp = tempfile::TempDir::new().unwrap();
        let nested = tmp.path().join("a/b/c");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(tmp.path().join("porter.toml"), "").unwrap();
        let found = Config::discover(&nested).unwrap();
        assert_eq!(found, tmp.path().join("porter.toml"));
    }
}
