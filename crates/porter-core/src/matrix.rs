//! Build the GitHub Actions job matrix that the release workflow fans
//! out from a `porter.toml`'s `[[artifacts]]` blocks.
//!
//! Each artifact entry expands to one or more matrix rows depending on its
//! kind: an `oci-image` is one row, a `cli-binary` expands to one row per
//! target triple, etc. The reusable `release.yml` consumes the JSON we
//! emit here as a `strategy.matrix.include` array.

use serde::Serialize;
use serde_json::Value;

use crate::config::{ArtifactConfig, Config};

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
        }
    }
}

pub fn build_matrix(config: &Config) -> Vec<MatrixRow> {
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
                rows.push(r);
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
                rows.push(r);
            }
            ArtifactConfig::NpmPackage {
                name,
                path,
                registry,
            } => {
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
                    rows.push(r);
                }
            }
        }
    }
    rows
}

/// Map a Rust target triple to a GitHub-hosted runner. Aarch64 macOS uses
/// the M-series runners, x86_64 macOS still uses Intel macs (macos-13 is
/// the last Intel image).
fn runner_for_target(target: &str) -> &'static str {
    match target {
        "x86_64-unknown-linux-gnu" => "ubuntu-latest",
        "aarch64-unknown-linux-gnu" => "ubuntu-24.04-arm",
        "x86_64-apple-darwin" => "macos-13",
        "aarch64-apple-darwin" => "macos-14",
        // Reasonable default; releases will fail loudly if this is wrong.
        _ => "ubuntu-latest",
    }
}

/// Render the matrix as a JSON object suitable for `strategy.matrix`,
/// i.e. `{"include": [...]}`. Empty matrices serialize to
/// `{"include": []}` which GH Actions treats as a no-op.
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
