use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow, bail};
use serde::Deserialize;
use serde::de::{self, Deserializer};

/// A parsed `porter.toml`.
///
/// The unit porter releases is a **group**: a set of components that share one
/// version number and move in lockstep. Each [`Component`] bundles an optional
/// version source (the file whose embedded version string is rewritten) and an
/// optional [`Artifact`] (what gets built and published). Groups are
/// independent — a changeset names every group it bumps, and each group cuts
/// its own tags off its own version line.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Config {
    #[serde(default)]
    pub changesets: ChangesetsConfig,
    /// Release lines. The TOML header is `[[group]]`.
    #[serde(rename = "group", default)]
    pub groups: Vec<Group>,
    /// Named registries an artifact's `registry` field can reference. The
    /// TOML header is `[registries.<name>]`.
    #[serde(default)]
    pub registries: BTreeMap<String, Registry>,
    #[serde(default)]
    pub signing: Option<SigningConfig>,
    #[serde(default)]
    pub attestation: Option<AttestationConfig>,
    #[serde(default)]
    pub release: ReleaseConfig,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ChangesetsConfig {
    #[serde(default = "default_changesets_dir")]
    pub directory: PathBuf,
}

impl Default for ChangesetsConfig {
    fn default() -> Self {
        Self {
            directory: default_changesets_dir(),
        }
    }
}

fn default_changesets_dir() -> PathBuf {
    PathBuf::from(".changeset")
}

/// One release line: a set of components pinned to a single shared version,
/// with its own changelog and its own tags.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    /// Changelog this group prepends its release sections to. Defaults to the
    /// repo-wide [`ReleaseConfig::changelog`] when unset; several groups may
    /// share one file (sections are prepended independently).
    #[serde(default)]
    pub changelog: Option<PathBuf>,
    #[serde(default)]
    pub components: Vec<Component>,
}

/// A single versioned thing — a version source, an artifact, or both.
///
/// The version source is where its version string lives; the artifact is how
/// it's built/published; at least one must be present. The `id` is the
/// component's identity — it names the artifact and is the stem of its tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    pub id: String,
    pub version: Option<VersionSource>,
    pub artifact: Option<Artifact>,
    /// Overrides the default `<id>/v` tag stem. porter's own component sets
    /// `"v"` to keep bare `v0.1.0` tags.
    pub tag_prefix: Option<String>,
}

/// A file whose embedded version string moves with its group. Loaded into a
/// [`crate::versioned_files::VersionedFile`] adapter that reads and rewrites
/// the concrete format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionSource {
    CargoWorkspace {
        path: PathBuf,
    },
    HelmChart {
        path: PathBuf,
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

impl VersionSource {
    /// Build a version source from the flat component fields. Returns a
    /// human-readable error (no serde span) describing the missing/extra field.
    fn from_raw(
        ty: &str,
        path: Option<PathBuf>,
        pattern: Option<String>,
        update_app_version: Option<bool>,
    ) -> Result<Self, String> {
        let path = path.ok_or_else(|| format!("version source `{ty}` requires `path`"))?;
        match ty {
            "cargo-workspace" => Ok(Self::CargoWorkspace { path }),
            "helm-chart" => Ok(Self::HelmChart {
                path,
                update_app_version: update_app_version.unwrap_or(true),
            }),
            "package-json" => Ok(Self::PackageJson { path }),
            "regex" => {
                let pattern = pattern
                    .ok_or_else(|| "version source `regex` requires `pattern`".to_owned())?;
                Ok(Self::Regex { path, pattern })
            }
            other => Err(format!(
                "unknown version source `type = {other:?}` (expected cargo-workspace, \
                 helm-chart, package-json, or regex)"
            )),
        }
    }

    /// Path to the file this source rewrites.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::CargoWorkspace { path }
            | Self::HelmChart { path, .. }
            | Self::PackageJson { path }
            | Self::Regex { path, .. } => path,
        }
    }
}

/// The flat shape a `[[group.components]]` inline table deserializes into. The
/// version-source fields sit alongside `id`/`artifact`/`tag_prefix`, so we
/// parse them flat and fold them into a typed [`VersionSource`] by hand —
/// `#[serde(flatten)]` onto an `Option<internally-tagged enum>` is unreliable.
#[derive(Deserialize)]
struct RawComponent {
    id: String,
    #[serde(default, rename = "type")]
    ty: Option<String>,
    #[serde(default)]
    path: Option<PathBuf>,
    #[serde(default)]
    pattern: Option<String>,
    #[serde(default)]
    update_app_version: Option<bool>,
    #[serde(default)]
    artifact: Option<Artifact>,
    #[serde(default)]
    tag_prefix: Option<String>,
}

impl<'de> Deserialize<'de> for Component {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawComponent::deserialize(deserializer)?;
        let version = if let Some(ty) = raw.ty.as_deref() {
            Some(
                VersionSource::from_raw(ty, raw.path, raw.pattern, raw.update_app_version)
                    .map_err(de::Error::custom)?,
            )
        } else {
            // No `type` means no version source; reject stray version fields
            // rather than silently dropping them.
            if raw.path.is_some() || raw.pattern.is_some() {
                return Err(de::Error::custom(format!(
                    "component {:?} has `path`/`pattern` but no `type`",
                    raw.id
                )));
            }
            None
        };
        Ok(Self {
            id: raw.id,
            version,
            artifact: raw.artifact,
            tag_prefix: raw.tag_prefix,
        })
    }
}

/// What a component builds and publishes. The component `id` supplies the
/// artifact's name, so no `name` field appears here.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Artifact {
    OciImage {
        context: PathBuf,
        dockerfile: PathBuf,
        registry: String,
        #[serde(default = "default_platforms")]
        platforms: Vec<String>,
        /// Optional repo-owned publish command. When set, the release workflow
        /// runs it INSTEAD of `docker/build-push-action`, with the image ref and
        /// build context exposed as `PORTER_*` env vars (and any `build-secrets`
        /// exported). The repo owns build args, secrets, and stages; the command
        /// must build *and* push to `$PORTER_IMAGE`. porter resolves the pushed
        /// digest (or reads `$PORTER_DIGEST_FILE`) and signs it — auth is still
        /// driven by the registry's declared kind, not the command.
        #[serde(default)]
        publish: Option<String>,
    },
    HelmChart {
        chart: PathBuf,
        registry: String,
    },
    NpmPackage {
        path: PathBuf,
        #[serde(default = "default_npm_registry")]
        registry: String,
    },
    PythonWheel {
        path: PathBuf,
    },
    CliBinary {
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

impl Artifact {
    /// The registry an artifact publishes to, if its kind has one. cli-binary
    /// (GitHub Release) and python-wheel (Release upload) have none.
    #[must_use]
    pub fn registry(&self) -> Option<&str> {
        match self {
            Self::OciImage { registry, .. }
            | Self::HelmChart { registry, .. }
            | Self::NpmPackage { registry, .. } => Some(registry),
            Self::PythonWheel { .. } | Self::CliBinary { .. } => None,
        }
    }

    /// The registry kind this artifact requires, used to reject a mismatched
    /// `[registries.<name>]` reference (e.g. an oci-image pointed at an npm
    /// registry).
    #[must_use]
    const fn expected_registry_kind(&self) -> Option<RegistryKind> {
        match self {
            Self::OciImage { .. } => Some(RegistryKind::Oci),
            Self::HelmChart { .. } => Some(RegistryKind::OciHelm),
            Self::NpmPackage { .. } => Some(RegistryKind::Npm),
            Self::PythonWheel { .. } | Self::CliBinary { .. } => None,
        }
    }
}

/// A named publish target.
///
/// An artifact's `registry` field is either a key into `[registries]` (resolved
/// to this, with auth) or, for back-compat, a bare URL used as-is with no auth.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Registry {
    pub kind: RegistryKind,
    pub url: String,
    #[serde(default)]
    pub auth: AuthConfig,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RegistryKind {
    /// Container images (oci-image artifacts).
    Oci,
    /// Helm charts pushed as OCI artifacts (helm-chart artifacts).
    OciHelm,
    /// JavaScript packages (npm-package artifacts).
    Npm,
    /// Python package index (reserved; python-wheel currently uploads to the
    /// GitHub Release rather than a registry).
    Pypi,
}

/// How CI authenticates to a registry.
///
/// Secrets are referenced by *name*; the release workflow looks the names up in
/// a single `registry-auth` JSON secret (GitHub Actions can't index the
/// `secrets` context by a dynamic key).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum AuthConfig {
    /// No credentials (anonymous, or the default for a bare-URL registry).
    #[default]
    None,
    /// The workflow's `GITHUB_TOKEN` — the common case for `ghcr.io`.
    GithubToken,
    /// Username/password (e.g. Docker Hub), each a secret name.
    Basic {
        username_secret: String,
        password_secret: String,
    },
    /// A single bearer token (e.g. a private npm registry), a secret name.
    Token { token_secret: String },
    /// AWS ECR via GitHub Actions OIDC. Unlike the others, `role_arn`/`region`
    /// are plain config values, not secret names: the release job's
    /// `id-token: write` mints the token, `aws-actions/configure-aws-credentials`
    /// assumes `role_arn`, and the login step runs `aws ecr get-login-password |
    /// docker login` (plus `helm registry login` for chart rows). Valid only on
    /// `oci`/`oci-helm` registries.
    AwsEcr { role_arn: String, region: String },
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
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SigningBackend {
    /// Keyless signing via Sigstore (Fulcio for certs, Rekor for the
    /// transparency log), using the CI runner's OIDC token.
    #[default]
    Sigstore,
    /// Disable signing entirely.
    None,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct AttestationConfig {
    pub layout: PathBuf,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ReleaseConfig {
    /// Default changelog for groups that don't set their own.
    #[serde(default = "default_changelog_path")]
    pub changelog: PathBuf,
    /// Template for the rolling "Version Packages" PR title (and its branch
    /// commit). Supports `{version}` — substituted when exactly one group
    /// bumps; with several groups moving at once there's no single version, so
    /// the CLI falls back to the literal stem before `{version}`.
    #[serde(default = "default_version_pr_title")]
    pub version_pr_title: String,
}

impl Default for ReleaseConfig {
    fn default() -> Self {
        Self {
            changelog: default_changelog_path(),
            version_pr_title: default_version_pr_title(),
        }
    }
}

/// Placeholder substituted in [`ReleaseConfig::version_pr_title`].
const VERSION_PLACEHOLDER: &str = "{version}";

impl ReleaseConfig {
    /// Render [`Self::version_pr_title`] for a single `version` string.
    #[must_use]
    pub fn render_pr_title(&self, version: &str) -> String {
        self.version_pr_title.replace(VERSION_PLACEHOLDER, version)
    }

    /// Render the title when several groups move at once: drop the
    /// `{version}` placeholder (there's no single version) and trim the
    /// dangling separator so `"Version Packages: {version}"` becomes
    /// `"Version Packages"`.
    #[must_use]
    pub fn render_pr_title_multi(&self) -> String {
        self.version_pr_title
            .replace(VERSION_PLACEHOLDER, "")
            .trim_end_matches([':', ' ', '-'])
            .to_owned()
    }
}

fn default_changelog_path() -> PathBuf {
    PathBuf::from("CHANGELOG.md")
}

fn default_version_pr_title() -> String {
    format!("Version Packages: {VERSION_PLACEHOLDER}")
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

    /// Parse and validate a `porter.toml` body.
    ///
    /// # Errors
    ///
    /// Returns an error if the input is not valid TOML, uses the removed
    /// `versioned_files`/`artifacts` tables, or fails [`Self::validate`].
    pub fn from_toml(body: &str) -> Result<Self> {
        // A pre-parse to a generic document lets us give a targeted migration
        // error for the pre-groups schema instead of silently ignoring the
        // unknown tables and then complaining that no groups are defined.
        let doc: toml::Value =
            toml::from_str(body).map_err(|e| anyhow!("invalid porter.toml: {e}"))?;
        if let Some(table) = doc.as_table()
            && (table.contains_key("versioned_files") || table.contains_key("artifacts"))
        {
            bail!(
                "porter.toml uses the removed top-level `versioned_files`/`artifacts` tables; \
                 declare `[[group]]` blocks whose `components` carry a version source and/or \
                 an `artifact` instead"
            );
        }
        // Flatten the toml error into the message so a component-level cause
        // (e.g. "version source `regex` requires `pattern`") surfaces rather
        // than being hidden one frame down under a generic context.
        let config: Self = toml::from_str(body).map_err(|e| anyhow!("invalid porter.toml: {e}"))?;
        config.validate()?;
        Ok(config)
    }

    /// Enforce the invariants the type system can't: a group to release into,
    /// unique group names, repo-wide-unique component ids (they're tag names),
    /// every component carrying a version source and/or an artifact, and every
    /// group owning at least one version source to read its current version
    /// from.
    ///
    /// # Errors
    ///
    /// Returns an error naming the first offending group or component.
    fn validate(&self) -> Result<()> {
        if self.groups.is_empty() {
            bail!("porter.toml defines no [[group]] blocks");
        }
        let mut seen_groups = BTreeSet::new();
        let mut seen_ids = BTreeSet::new();
        for group in &self.groups {
            if !seen_groups.insert(group.name.as_str()) {
                bail!("duplicate group name {:?}", group.name);
            }
            if group.components.is_empty() {
                bail!("group {:?} has no components", group.name);
            }
            let mut has_version_source = false;
            for component in &group.components {
                if !seen_ids.insert(component.id.as_str()) {
                    bail!(
                        "duplicate component id {:?} (ids are tag names and must be unique)",
                        component.id
                    );
                }
                if component.version.is_none() && component.artifact.is_none() {
                    bail!(
                        "component {:?} has neither a version source nor an artifact",
                        component.id
                    );
                }
                // A registry *reference* that names a `[registries]` entry must
                // be of a kind the artifact can publish to. An unrecognized
                // reference is a bare URL and is left alone.
                if let Some(artifact) = &component.artifact
                    && let Some(reference) = artifact.registry()
                    && let Some(registry) = self.registries.get(reference)
                    && let Some(expected) = artifact.expected_registry_kind()
                    && registry.kind != expected
                {
                    bail!(
                        "component {:?} references registry {:?} of kind {:?}, \
                         but its artifact needs a {:?} registry",
                        component.id,
                        reference,
                        registry.kind,
                        expected
                    );
                }
                has_version_source |= component.version.is_some();
            }
            if !has_version_source {
                bail!(
                    "group {:?} has no version-bearing component to read its current version from",
                    group.name
                );
            }
        }
        // `aws-ecr` auth is an OCI-registry concern: ECR serves container images
        // and OCI-packaged Helm charts, not npm/pypi.
        for (name, registry) in &self.registries {
            if matches!(registry.auth, AuthConfig::AwsEcr { .. })
                && !matches!(registry.kind, RegistryKind::Oci | RegistryKind::OciHelm)
            {
                bail!(
                    "registry {name:?} uses aws-ecr auth but its kind is {:?}; \
                     aws-ecr is only valid for oci/oci-helm registries",
                    registry.kind
                );
            }
        }
        Ok(())
    }

    /// Resolve the effective signing configuration. Signing is opt-in: an
    /// absent `[signing]` block resolves to disabled, so a consumer gets
    /// signing only once they add the block (no other config required).
    #[must_use]
    pub fn signing(&self) -> SigningConfig {
        self.signing.clone().unwrap_or_else(SigningConfig::disabled)
    }

    /// The group with this name, if any.
    #[must_use]
    pub fn group(&self, name: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.name == name)
    }

    /// The named registry `reference` resolves to. `None` means the reference
    /// is a bare URL used as-is (anonymous, no auth).
    #[must_use]
    pub fn registry(&self, reference: &str) -> Option<&Registry> {
        self.registries.get(reference)
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
            [[group]]
            name = "default"
            components = [
              { id = "porter", type = "cargo-workspace", path = "Cargo.toml" },
            ]
        "#};
        let cfg = Config::from_toml(body).unwrap();
        assert_eq!(cfg.groups.len(), 1);
        let c = &cfg.groups[0].components[0];
        assert_eq!(c.id, "porter");
        assert!(matches!(
            c.version,
            Some(VersionSource::CargoWorkspace { .. })
        ));
        assert_eq!(cfg.changesets.directory, PathBuf::from(".changeset"));
    }

    #[test]
    fn parses_multi_group_config_with_unified_components() {
        let body = indoc! {r#"
            [[group]]
            name = "sdk"
            changelog = "python/CHANGELOG.md"
            components = [
              { id = "py-sdk", type = "regex", path = "py/pyproject.toml",
                pattern = '(?m)^version = "(?P<version>[^"]+)"',
                artifact = { kind = "python-wheel", path = "py" } },
              { id = "ts-sdk", type = "package-json", path = "ts/package.json",
                artifact = { kind = "npm-package", path = "ts" } },
            ]

            [[group]]
            name = "default"
            components = [
              { id = "porter", type = "cargo-workspace", path = "Cargo.toml", tag_prefix = "v",
                artifact = { kind = "cli-binary", package = "porter-cli" } },
              { id = "api", artifact = { kind = "oci-image", context = ".",
                dockerfile = "Dockerfile", registry = "ghcr.io/x/api" } },
            ]
        "#};
        let cfg = Config::from_toml(body).unwrap();
        assert_eq!(cfg.groups.len(), 2);
        let sdk = &cfg.groups[0];
        assert_eq!(sdk.changelog, Some(PathBuf::from("python/CHANGELOG.md")));
        assert_eq!(sdk.components.len(), 2);
        // artifact-only component: version is None, artifact present.
        let api = &cfg.groups[1].components[1];
        assert_eq!(api.id, "api");
        assert!(api.version.is_none());
        assert!(matches!(api.artifact, Some(Artifact::OciImage { .. })));
        // tag_prefix override parsed.
        assert_eq!(cfg.groups[1].components[0].tag_prefix.as_deref(), Some("v"));
    }

    #[test]
    fn rejects_legacy_versioned_files() {
        let body = indoc! {r#"
            [[versioned_files]]
            type = "cargo-workspace"
            path = "Cargo.toml"
        "#};
        let err = Config::from_toml(body).unwrap_err().to_string();
        assert!(err.contains("versioned_files"), "{err}");
        assert!(err.contains("[[group]]"), "{err}");
    }

    #[test]
    fn rejects_duplicate_component_ids() {
        let body = indoc! {r#"
            [[group]]
            name = "a"
            components = [ { id = "x", type = "cargo-workspace", path = "Cargo.toml" } ]

            [[group]]
            name = "b"
            components = [ { id = "x", type = "package-json", path = "package.json" } ]
        "#};
        let err = Config::from_toml(body).unwrap_err().to_string();
        assert!(err.contains("duplicate component id"), "{err}");
    }

    #[test]
    fn rejects_component_with_neither_version_nor_artifact() {
        let body = indoc! {r#"
            [[group]]
            name = "a"
            components = [ { id = "x" } ]
        "#};
        let err = Config::from_toml(body).unwrap_err().to_string();
        assert!(
            err.contains("neither a version source nor an artifact"),
            "{err}"
        );
    }

    #[test]
    fn rejects_group_with_no_version_source() {
        let body = indoc! {r#"
            [[group]]
            name = "a"
            components = [
              { id = "img", artifact = { kind = "oci-image", context = ".",
                dockerfile = "Dockerfile", registry = "r" } },
            ]
        "#};
        let err = Config::from_toml(body).unwrap_err().to_string();
        assert!(err.contains("no version-bearing component"), "{err}");
    }

    #[test]
    fn regex_without_pattern_errors() {
        let body = indoc! {r#"
            [[group]]
            name = "a"
            components = [ { id = "x", type = "regex", path = "f" } ]
        "#};
        let err = Config::from_toml(body).unwrap_err().to_string();
        assert!(err.contains("requires `pattern`"), "{err}");
    }

    #[test]
    fn named_registry_parses_with_auth() {
        let cfg = Config::from_toml(indoc! {r#"
            [registries.dockerhub]
            kind = "oci"
            url = "docker.io/tractorbeam"
            auth = { type = "basic", username_secret = "DH_USER", password_secret = "DH_PAT" }

            [[group]]
            name = "default"
            components = [
              { id = "api", type = "cargo-workspace", path = "Cargo.toml",
                artifact = { kind = "oci-image", context = ".", dockerfile = "Dockerfile",
                  registry = "dockerhub" } },
            ]
        "#})
        .unwrap();
        let reg = cfg.registry("dockerhub").unwrap();
        assert_eq!(reg.kind, RegistryKind::Oci);
        assert_eq!(reg.url, "docker.io/tractorbeam");
        assert!(matches!(reg.auth, AuthConfig::Basic { .. }));
    }

    #[test]
    fn aws_ecr_auth_parses() {
        let cfg = Config::from_toml(indoc! {r#"
            [registries.ecr]
            kind = "oci"
            url = "575108936009.dkr.ecr.us-east-1.amazonaws.com/tractorbeam"
            auth = { type = "aws-ecr", role_arn = "arn:aws:iam::575108936009:role/gha", region = "us-east-1" }

            [[group]]
            name = "default"
            components = [
              { id = "api", type = "cargo-workspace", path = "Cargo.toml",
                artifact = { kind = "oci-image", context = ".", dockerfile = "Dockerfile",
                  registry = "ecr" } },
            ]
        "#})
        .unwrap();
        let reg = cfg.registry("ecr").unwrap();
        assert!(matches!(
            &reg.auth,
            AuthConfig::AwsEcr { role_arn, region }
                if role_arn == "arn:aws:iam::575108936009:role/gha" && region == "us-east-1"
        ));
    }

    #[test]
    fn aws_ecr_rejected_on_npm_registry() {
        let body = indoc! {r#"
            [registries.bad]
            kind = "npm"
            url = "https://registry.npmjs.org"
            auth = { type = "aws-ecr", role_arn = "arn:aws:iam::1:role/x", region = "us-east-1" }

            [[group]]
            name = "default"
            components = [ { id = "x", type = "cargo-workspace", path = "Cargo.toml" } ]
        "#};
        let err = Config::from_toml(body).unwrap_err().to_string();
        assert!(err.contains("aws-ecr is only valid"), "{err}");
    }

    #[test]
    fn registry_kind_mismatch_errors() {
        let body = indoc! {r#"
            [registries.npmjs]
            kind = "npm"
            url = "https://registry.npmjs.org"

            [[group]]
            name = "default"
            components = [
              { id = "api", type = "cargo-workspace", path = "Cargo.toml",
                artifact = { kind = "oci-image", context = ".", dockerfile = "Dockerfile",
                  registry = "npmjs" } },
            ]
        "#};
        let err = Config::from_toml(body).unwrap_err().to_string();
        assert!(
            err.contains("needs a Oci registry") || err.contains("kind"),
            "{err}"
        );
    }

    #[test]
    fn signing_off_when_block_absent() {
        let cfg = Config::from_toml(indoc! {r#"
            [[group]]
            name = "default"
            components = [ { id = "x", type = "cargo-workspace", path = "Cargo.toml" } ]
        "#})
        .unwrap();
        assert!(cfg.signing.is_none());
        assert!(!cfg.signing().enabled());
    }

    #[test]
    fn empty_signing_block_enables_public_sigstore() {
        let cfg = Config::from_toml(indoc! {r#"
            [signing]

            [[group]]
            name = "default"
            components = [ { id = "x", type = "cargo-workspace", path = "Cargo.toml" } ]
        "#})
        .unwrap();
        let signing = cfg.signing();
        assert!(signing.enabled());
        assert_eq!(signing.backend, SigningBackend::Sigstore);
        assert_eq!(signing.fulcio_url, "https://fulcio.sigstore.dev");
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

    #[test]
    fn default_pr_title_preserves_legacy_format() {
        let cfg = ReleaseConfig::default();
        assert_eq!(cfg.render_pr_title("0.1.1"), "Version Packages: 0.1.1");
        assert_eq!(cfg.render_pr_title_multi(), "Version Packages");
    }
}
