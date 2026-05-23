use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
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

const fn default_true() -> bool {
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

/// How releases are signed.
///
/// Signing is opt-in: with no `[signing]` block, releases aren't signed
/// (see [`Config::signing`]). Adding the block — empty is enough — turns
/// on keyless Sigstore for *every* signable artifact (oci-image,
/// helm-chart, cli-binary), each getting a signature plus a SLSA
/// provenance attestation. `backend = "none"` is an explicit off-switch
/// for keeping the block while disabling. This struct's [`Default`] is
/// exactly what an empty `[signing]` block parses to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SigningConfig {
    #[serde(default)]
    pub backend: SigningBackend,
    #[serde(default = "default_fulcio")]
    pub fulcio_url: String,
    #[serde(default = "default_rekor")]
    pub rekor_url: String,
}

impl Default for SigningConfig {
    fn default() -> Self {
        Self {
            backend: SigningBackend::default(),
            fulcio_url: default_fulcio(),
            rekor_url: default_rekor(),
        }
    }
}

impl SigningConfig {
    /// The config used when there's no `[signing]` block: signing off.
    #[must_use]
    fn disabled() -> Self {
        Self {
            backend: SigningBackend::None,
            ..Self::default()
        }
    }

    /// Whether signing should run. False only when the backend is
    /// `none` (either explicit or the absent-block default).
    #[must_use]
    pub const fn enabled(&self) -> bool {
        !matches!(self.backend, SigningBackend::None)
    }
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
    /// Keyless signing via Sigstore (Fulcio for certs, Rekor for the
    /// transparency log), using the CI runner's OIDC token.
    #[default]
    Sigstore,
    /// Disable signing entirely.
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

    /// Read and parse a `porter.toml` from disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or its contents fail
    /// to parse as a valid porter config.
    pub fn load(path: &Path) -> Result<Self> {
        let body = fs::read_to_string(path)
            .with_context(|| format!("reading porter config {}", path.display()))?;
        Self::from_toml(&body).with_context(|| format!("parsing porter config {}", path.display()))
    }

    /// Parse a `porter.toml` body from a string.
    ///
    /// # Errors
    ///
    /// Returns an error if the input is not valid TOML or fails schema
    /// validation.
    pub fn from_toml(body: &str) -> Result<Self> {
        toml::from_str(body).context("invalid porter.toml")
    }

    /// Resolve the effective signing configuration. Signing is opt-in: an
    /// absent `[signing]` block resolves to disabled, so a consumer gets
    /// signing only once they add the block (no other config required).
    #[must_use]
    pub fn signing(&self) -> SigningConfig {
        self.signing.clone().unwrap_or_else(SigningConfig::disabled)
    }

    /// Find `porter.toml` by walking up from `start`.
    #[must_use]
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
    fn signing_off_when_block_absent() {
        let cfg = Config::from_toml("").unwrap();
        assert!(cfg.signing.is_none());
        assert!(!cfg.signing().enabled());
    }

    #[test]
    fn empty_signing_block_enables_public_sigstore() {
        // Zero config beyond declaring the block: signing turns on with
        // public Sigstore endpoints.
        let cfg = Config::from_toml("[signing]\n").unwrap();
        let signing = cfg.signing();
        assert!(signing.enabled());
        assert_eq!(signing.backend, SigningBackend::Sigstore);
        assert_eq!(signing.fulcio_url, "https://fulcio.sigstore.dev");
        assert_eq!(signing.rekor_url, "https://rekor.sigstore.dev");
    }

    #[test]
    fn signing_backend_none_disables() {
        let cfg = Config::from_toml(indoc! {r#"
            [signing]
            backend = "none"
        "#})
        .unwrap();
        assert!(!cfg.signing().enabled());
    }

    #[test]
    fn signing_custom_urls_parse() {
        let cfg = Config::from_toml(indoc! {r#"
            [signing]
            backend = "sigstore"
            fulcio_url = "https://fulcio.internal.example"
            rekor_url = "https://rekor.internal.example"
        "#})
        .unwrap();
        let signing = cfg.signing();
        assert!(signing.enabled());
        assert_eq!(signing.fulcio_url, "https://fulcio.internal.example");
        assert_eq!(signing.rekor_url, "https://rekor.internal.example");
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
