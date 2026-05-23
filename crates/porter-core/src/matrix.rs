//! Build the GitHub Actions job matrix that the release workflow fans
//! out from a `porter.toml`'s `[[artifacts]]` blocks.
//!
//! Each artifact entry expands to one or more matrix rows depending on its
//! kind: an `oci-image` is one row, a `cli-binary` expands to one row per
//! target triple, etc. The reusable `release.yml` consumes the JSON we
//! emit here as a `strategy.matrix.include` array.
//!
//! Signing config travels on the matrix too: when a repo's `[signing]`
//! is enabled, each signable row carries `sign = true` and the
//! Fulcio/Rekor endpoints, and `release.yml` gates its cosign steps on
//! `matrix.sign`.

use serde::Serialize;
use serde_json::Value;

use crate::config::{ArtifactConfig, Config, SigningConfig};

/// One row in the `strategy.matrix.include` array. Carries the union of
/// every field any kind of artifact needs; downstream `if:` conditions
/// pick the right job step based on `kind`.
#[derive(Debug, Clone, Serialize)]
pub struct MatrixRow {
    /// Stable identifier for the row. Used as the GH Actions job name.
    pub id: String,
    pub kind: String,
    pub name: String,

    // Common fields surfaced as Option<...> so absent ones serialize to
    // `null` and the workflow can branch on them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dockerfile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platforms: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chart: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// GitHub-hosted runner label, picked from `target` for cli-binary
    /// rows and `linux/$arch` for oci-image rows. The workflow uses
    /// `runs-on: ${{ matrix.runner }}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner: Option<String>,

    // Set on signable rows when signing is enabled; absent means the row
    // isn't signed. `release.yml` gates its cosign steps on `matrix.sign`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sign: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fulcio_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rekor_url: Option<String>,
}

impl MatrixRow {
    fn base(kind: &str, name: &str, suffix: &str) -> Self {
        let id = if suffix.is_empty() {
            format!("{kind}-{name}")
        } else {
            format!("{kind}-{name}-{suffix}")
        };
        Self {
            id,
            kind: kind.into(),
            name: name.into(),
            registry: None,
            context: None,
            dockerfile: None,
            platforms: None,
            chart: None,
            path: None,
            package: None,
            target: None,
            runner: None,
            sign: None,
            fulcio_url: None,
            rekor_url: None,
        }
    }

    /// Stamp this row with signing metadata from `signing`, but only if
    /// `signing` is enabled. Called for signable kinds (oci-image,
    /// helm-chart, cli-binary); npm/python rows are left unsigned because
    /// those ecosystems carry their own provenance mechanisms.
    fn with_signing(mut self, signing: &SigningConfig) -> Self {
        if signing.enabled() {
            self.sign = Some(true);
            self.fulcio_url = Some(signing.fulcio_url.clone());
            self.rekor_url = Some(signing.rekor_url.clone());
        }
        self
    }
}

#[must_use]
pub fn build_matrix(config: &Config) -> Vec<MatrixRow> {
    let signing = config.signing();
    let mut rows = Vec::new();
    for art in &config.artifacts {
        match art {
            ArtifactConfig::OciImage {
                name,
                context,
                dockerfile,
                registry,
                platforms,
            } => {
                let mut r = MatrixRow::base("oci-image", name, "");
                r.context = Some(context.display().to_string());
                r.dockerfile = Some(dockerfile.display().to_string());
                r.registry = Some(registry.clone());
                r.platforms = Some(platforms.join(","));
                r.runner = Some("ubuntu-latest".into());
                rows.push(r.with_signing(&signing));
            }
            ArtifactConfig::HelmChart {
                name,
                chart,
                registry,
            } => {
                let mut r = MatrixRow::base("helm-chart", name, "");
                r.chart = Some(chart.display().to_string());
                r.registry = Some(registry.clone());
                r.runner = Some("ubuntu-latest".into());
                rows.push(r.with_signing(&signing));
            }
            ArtifactConfig::NpmPackage {
                name,
                path,
                registry,
            } => {
                // npm packages carry their own provenance (`npm publish
                // --provenance`); porter doesn't cosign-sign them.
                let mut r = MatrixRow::base("npm-package", name, "");
                r.path = Some(path.display().to_string());
                r.registry = Some(registry.clone());
                r.runner = Some("ubuntu-latest".into());
                rows.push(r);
            }
            ArtifactConfig::PythonWheel { name, path } => {
                let mut r = MatrixRow::base("python-wheel", name, "");
                r.path = Some(path.display().to_string());
                r.runner = Some("ubuntu-latest".into());
                rows.push(r);
            }
            ArtifactConfig::CliBinary {
                name,
                package,
                targets,
            } => {
                for target in targets {
                    let mut r = MatrixRow::base("cli-binary", name, target);
                    r.package = Some(package.clone());
                    r.target = Some(target.clone());
                    r.runner = Some(runner_for_target(target).into());
                    rows.push(r.with_signing(&signing));
                }
            }
        }
    }
    rows
}

/// Map a Rust target triple to a GitHub-hosted runner. Aarch64 macOS uses
/// the M-series runners; `x86_64` macOS uses `macos-15-intel` (the
/// canonical Intel macOS label since the `macos-13` image was
/// deprecated).
fn runner_for_target(target: &str) -> &'static str {
    match target {
        "aarch64-unknown-linux-gnu" => "ubuntu-24.04-arm",
        "x86_64-apple-darwin" => "macos-15-intel",
        "aarch64-apple-darwin" => "macos-14",
        // x86_64 Linux is the default. Any unknown target also lands
        // here so releases at least attempt to build; CI will fail
        // loudly if the runner can't compile for the target.
        _ => "ubuntu-latest",
    }
}

/// Render the matrix as a JSON object suitable for `strategy.matrix`,
/// i.e. `{"include": [...]}`. Empty matrices serialize to
/// `{"include": []}` which GH Actions treats as a no-op.
#[must_use]
pub fn render_for_actions(rows: &[MatrixRow]) -> Value {
    serde_json::json!({ "include": rows })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use indoc::indoc;

    #[test]
    fn empty_artifacts_yields_empty_matrix() {
        let cfg = Config::from_toml("").unwrap();
        let m = build_matrix(&cfg);
        assert!(m.is_empty());
    }

    #[test]
    fn cli_binary_fans_out_per_target() {
        let cfg = Config::from_toml(indoc! {r#"
            [[artifacts]]
            kind = "cli-binary"
            name = "porter"
            package = "porter-cli"
            targets = ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"]
        "#})
        .unwrap();
        let m = build_matrix(&cfg);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].id, "cli-binary-porter-x86_64-unknown-linux-gnu");
        assert_eq!(m[0].target.as_deref(), Some("x86_64-unknown-linux-gnu"));
        assert_eq!(m[0].runner.as_deref(), Some("ubuntu-latest"));
        assert_eq!(m[1].id, "cli-binary-porter-aarch64-apple-darwin");
        assert_eq!(m[1].runner.as_deref(), Some("macos-14"));
    }

    #[test]
    fn oci_image_serializes_platforms() {
        let cfg = Config::from_toml(indoc! {r#"
            [[artifacts]]
            kind = "oci-image"
            name = "api"
            context = "rust/"
            dockerfile = "rust/bins/api/Dockerfile"
            registry = "ghcr.io/example/api"
        "#})
        .unwrap();
        let m = build_matrix(&cfg);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].kind, "oci-image");
        assert_eq!(m[0].platforms.as_deref(), Some("linux/amd64,linux/arm64"));
        assert_eq!(m[0].registry.as_deref(), Some("ghcr.io/example/api"));
    }

    #[test]
    fn render_wraps_in_include() {
        let cfg = Config::from_toml(indoc! {r#"
            [[artifacts]]
            kind = "helm-chart"
            name = "platform"
            chart = "deploy/helm/platform"
            registry = "oci://ghcr.io/example/charts"
        "#})
        .unwrap();
        let m = build_matrix(&cfg);
        let v = render_for_actions(&m);
        assert!(v.get("include").is_some());
        assert_eq!(v["include"].as_array().unwrap().len(), 1);
        assert_eq!(v["include"][0]["kind"], "helm-chart");
    }

    #[test]
    fn no_signing_block_leaves_all_rows_unsigned() {
        // Opt-in: without a [signing] block, even signable kinds are
        // left unsigned.
        let cfg = Config::from_toml(indoc! {r#"
            [[artifacts]]
            kind = "oci-image"
            name = "api"
            context = "rust/"
            dockerfile = "rust/bins/api/Dockerfile"
            registry = "ghcr.io/example/api"

            [[artifacts]]
            kind = "cli-binary"
            name = "porter"
            package = "porter-cli"
            targets = ["x86_64-unknown-linux-gnu"]
        "#})
        .unwrap();
        for r in &build_matrix(&cfg) {
            assert_eq!(r.sign, None, "{} must not be signed", r.id);
            assert!(r.fulcio_url.is_none());
        }
    }

    #[test]
    fn signing_block_stamps_signable_rows() {
        let cfg = Config::from_toml(indoc! {r#"
            [signing]

            [[artifacts]]
            kind = "oci-image"
            name = "api"
            context = "rust/"
            dockerfile = "rust/bins/api/Dockerfile"
            registry = "ghcr.io/example/api"

            [[artifacts]]
            kind = "helm-chart"
            name = "platform"
            chart = "deploy/helm/platform"
            registry = "oci://ghcr.io/example/charts"

            [[artifacts]]
            kind = "cli-binary"
            name = "porter"
            package = "porter-cli"
            targets = ["x86_64-unknown-linux-gnu"]
        "#})
        .unwrap();
        let m = build_matrix(&cfg);
        for r in &m {
            assert_eq!(r.sign, Some(true), "{} should be signed", r.id);
            assert_eq!(r.fulcio_url.as_deref(), Some("https://fulcio.sigstore.dev"));
            assert_eq!(r.rekor_url.as_deref(), Some("https://rekor.sigstore.dev"));
        }
    }

    #[test]
    fn npm_and_python_rows_are_never_signed_even_when_enabled() {
        let cfg = Config::from_toml(indoc! {r#"
            [signing]

            [[artifacts]]
            kind = "npm-package"
            name = "sdk"
            path = "ts/packages/sdk"

            [[artifacts]]
            kind = "python-wheel"
            name = "client"
            path = "py/client"
        "#})
        .unwrap();
        let m = build_matrix(&cfg);
        for r in &m {
            assert_eq!(r.sign, None, "{} must not be signed", r.id);
            assert!(r.fulcio_url.is_none());
        }
    }

    #[test]
    fn signing_disabled_leaves_rows_unsigned() {
        let cfg = Config::from_toml(indoc! {r#"
            [signing]
            backend = "none"

            [[artifacts]]
            kind = "oci-image"
            name = "api"
            context = "rust/"
            dockerfile = "rust/bins/api/Dockerfile"
            registry = "ghcr.io/example/api"
        "#})
        .unwrap();
        let m = build_matrix(&cfg);
        assert_eq!(m[0].sign, None);
        assert!(m[0].fulcio_url.is_none());
    }

    #[test]
    fn custom_sigstore_urls_thread_into_rows() {
        let cfg = Config::from_toml(indoc! {r#"
            [signing]
            fulcio_url = "https://fulcio.internal.example"
            rekor_url = "https://rekor.internal.example"

            [[artifacts]]
            kind = "cli-binary"
            name = "porter"
            package = "porter-cli"
            targets = ["x86_64-unknown-linux-gnu"]
        "#})
        .unwrap();
        let m = build_matrix(&cfg);
        assert_eq!(
            m[0].fulcio_url.as_deref(),
            Some("https://fulcio.internal.example")
        );
        assert_eq!(
            m[0].rekor_url.as_deref(),
            Some("https://rekor.internal.example")
        );
    }

    #[test]
    fn signing_fields_serialize_only_when_present() {
        let cfg = Config::from_toml(indoc! {r#"
            [[artifacts]]
            kind = "npm-package"
            name = "sdk"
            path = "ts/packages/sdk"
        "#})
        .unwrap();
        let v = render_for_actions(&build_matrix(&cfg));
        let row = &v["include"][0];
        assert!(row.get("sign").is_none());
        assert!(row.get("fulcio_url").is_none());
    }

    #[test]
    fn unknown_target_falls_back_to_ubuntu() {
        let cfg = Config::from_toml(indoc! {r#"
            [[artifacts]]
            kind = "cli-binary"
            name = "porter"
            package = "porter-cli"
            targets = ["riscv64gc-unknown-linux-gnu"]
        "#})
        .unwrap();
        let m = build_matrix(&cfg);
        assert_eq!(m[0].runner.as_deref(), Some("ubuntu-latest"));
    }
}
