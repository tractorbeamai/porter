//! Build the GitHub Actions job matrix that the release workflow fans out
//! from a `porter.toml`'s groups.
//!
//! Each artifact-bearing component expands to one or more matrix rows
//! depending on its kind: an `oci-image` is one row, a `cli-binary` expands to
//! one row per target triple, etc. Every row carries its component's `tag`
//! (the per-component tag at its group's release version) and `group`, so the
//! reusable `release.yml` tags each artifact independently. The workflow
//! consumes the JSON we emit here as a `strategy.matrix.include` array.
//!
//! Signing config travels on the matrix too: when a repo's `[signing]` is
//! enabled, each signable row carries `sign = true` and the Fulcio/Rekor
//! endpoints, and `release.yml` gates its cosign steps on `matrix.sign`.

use std::collections::BTreeMap;

use semver::Version;
use serde::Serialize;
use serde_json::Value;

use crate::config::{Artifact, AuthConfig, Config, SigningConfig};

/// One row in the `strategy.matrix.include` array. Carries the union of every
/// field any kind of artifact needs; downstream `if:` conditions pick the
/// right job step based on `kind`.
#[derive(Debug, Clone, Serialize)]
pub struct MatrixRow {
    /// Stable identifier for the row. Used as the GH Actions job name.
    pub id: String,
    pub kind: String,
    /// The component id (the artifact's public name).
    pub name: String,
    /// The group this component releases in.
    pub group: String,
    /// The tag this artifact is published under, e.g. `py-sdk/v0.4.1`.
    pub tag: String,
    /// The bare version the group released, e.g. `0.4.1` — what the publish
    /// steps tag images/charts with (the git `tag` is for the release, not the
    /// artifact label).
    pub version: String,

    // ----- registry auth -------------------------------------------------
    // How the workflow logs in to `registry`. `auth_kind` is one of
    // github-token / basic / token / none; the `*_secret` fields name keys in
    // the workflow's `registry-auth` JSON secret (Actions can't index the
    // `secrets` context by a dynamic key, so creds arrive as one JSON blob).
    pub auth_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username_secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_secret: Option<String>,

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
    /// GitHub-hosted runner label, picked from `target` for cli-binary rows
    /// and `ubuntu-latest` otherwise.
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
    fn base(kind: &str, name: &str, group: &str, tag: &str, version: &str, suffix: &str) -> Self {
        let id = if suffix.is_empty() {
            format!("{kind}-{name}")
        } else {
            format!("{kind}-{name}-{suffix}")
        };
        Self {
            id,
            kind: kind.into(),
            name: name.into(),
            group: group.into(),
            tag: tag.into(),
            version: version.into(),
            auth_kind: "none".into(),
            username_secret: None,
            password_secret: None,
            token_secret: None,
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

    /// Stamp this row with the registry's auth method and the secret names CI
    /// should read for it.
    fn with_auth(mut self, auth: &AuthConfig) -> Self {
        match auth {
            AuthConfig::None => {}
            AuthConfig::GithubToken => self.auth_kind = "github-token".into(),
            AuthConfig::Basic {
                username_secret,
                password_secret,
            } => {
                self.auth_kind = "basic".into();
                self.username_secret = Some(username_secret.clone());
                self.password_secret = Some(password_secret.clone());
            }
            AuthConfig::Token { token_secret } => {
                self.auth_kind = "token".into();
                self.token_secret = Some(token_secret.clone());
            }
        }
        self
    }
}

/// Auth for a bare-URL registry reference (one that names no `[registries]`
/// entry): none.
const NO_AUTH: AuthConfig = AuthConfig::None;

/// Resolve an artifact's `registry` reference to a publish URL and its auth. A
/// reference that names a `[registries]` entry resolves to that entry; anything
/// else is treated as a bare URL used as-is, with no auth.
fn resolve<'a>(config: &'a Config, reference: &'a str) -> (String, &'a AuthConfig) {
    config.registry(reference).map_or_else(
        || (reference.to_owned(), &NO_AUTH),
        |r| (r.url.clone(), &r.auth),
    )
}

/// Expand every group's artifact-bearing components into matrix rows.
///
/// `versions` maps each group name to the version its artifacts are published
/// under (typically [`crate::apply::current_versions`] read from the release
/// commit). Groups absent from `versions` are skipped.
#[must_use]
pub fn build_matrix(config: &Config, versions: &BTreeMap<String, Version>) -> Vec<MatrixRow> {
    let signing = config.signing();
    let mut rows = Vec::new();
    for group in &config.groups {
        let Some(version) = versions.get(&group.name) else {
            continue;
        };
        let ver = version.to_string();
        for component in group.artifact_components() {
            let Some(artifact) = component.artifact() else {
                continue;
            };
            let id = component.id.as_str();
            let tag = component.tag(version);
            let g = group.name.as_str();
            match artifact {
                Artifact::OciImage {
                    context,
                    dockerfile,
                    registry,
                    platforms,
                } => {
                    // A named registry holds the org/host prefix; the image repo
                    // is `<url>/<id>`. A bare URL is used as the full repo.
                    let named = config.registry(registry);
                    let repo = named.map_or_else(
                        || registry.clone(),
                        |r| format!("{}/{id}", r.url.trim_end_matches('/')),
                    );
                    let auth = named.map_or(&NO_AUTH, |r| &r.auth);
                    let mut r = MatrixRow::base("oci-image", id, g, &tag, &ver, "");
                    r.context = Some(context.display().to_string());
                    r.dockerfile = Some(dockerfile.display().to_string());
                    r.registry = Some(repo);
                    r.platforms = Some(platforms.join(","));
                    r.runner = Some("ubuntu-latest".into());
                    rows.push(r.with_signing(&signing).with_auth(auth));
                }
                Artifact::HelmChart { chart, registry } => {
                    let (url, auth) = resolve(config, registry);
                    let mut r = MatrixRow::base("helm-chart", id, g, &tag, &ver, "");
                    r.chart = Some(chart.display().to_string());
                    r.registry = Some(url);
                    r.runner = Some("ubuntu-latest".into());
                    rows.push(r.with_signing(&signing).with_auth(auth));
                }
                Artifact::NpmPackage { path, registry } => {
                    // npm packages carry their own provenance (`npm publish
                    // --provenance`); porter doesn't cosign-sign them.
                    let (url, auth) = resolve(config, registry);
                    let mut r = MatrixRow::base("npm-package", id, g, &tag, &ver, "");
                    r.path = Some(path.display().to_string());
                    r.registry = Some(url);
                    r.runner = Some("ubuntu-latest".into());
                    rows.push(r.with_auth(auth));
                }
                Artifact::PythonWheel { path } => {
                    let mut r = MatrixRow::base("python-wheel", id, g, &tag, &ver, "");
                    r.path = Some(path.display().to_string());
                    r.runner = Some("ubuntu-latest".into());
                    rows.push(r);
                }
                Artifact::CliBinary { package, targets } => {
                    for target in targets {
                        let mut r = MatrixRow::base("cli-binary", id, g, &tag, &ver, target);
                        r.package = Some(package.clone());
                        r.target = Some(target.clone());
                        r.runner = Some(runner_for_target(target).into());
                        rows.push(r.with_signing(&signing));
                    }
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

/// Render the matrix as a JSON object suitable for `strategy.matrix`, i.e.
/// `{"include": [...]}`. Empty matrices serialize to `{"include": []}` which
/// GH Actions treats as a no-op.
#[must_use]
pub fn render_for_actions(rows: &[MatrixRow]) -> Value {
    serde_json::json!({ "include": rows })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use indoc::indoc;

    fn versions(pairs: &[(&str, &str)]) -> BTreeMap<String, Version> {
        pairs
            .iter()
            .map(|(g, v)| ((*g).to_owned(), Version::parse(v).unwrap()))
            .collect()
    }

    #[test]
    fn empty_artifacts_yields_empty_matrix() {
        let cfg = Config::from_toml(indoc! {r#"
            [[group]]
            name = "default"
            components = [ { id = "x", type = "cargo-workspace", path = "Cargo.toml" } ]
        "#})
        .unwrap();
        let m = build_matrix(&cfg, &versions(&[("default", "0.1.0")]));
        assert!(m.is_empty());
    }

    #[test]
    fn cli_binary_fans_out_per_target_and_carries_tag() {
        let cfg = Config::from_toml(indoc! {r#"
            [[group]]
            name = "default"
            components = [
              { id = "porter", type = "cargo-workspace", path = "Cargo.toml", tag_prefix = "v",
                artifact = { kind = "cli-binary", package = "porter-cli",
                  targets = ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"] } },
            ]
        "#})
        .unwrap();
        let m = build_matrix(&cfg, &versions(&[("default", "0.1.0")]));
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].id, "cli-binary-porter-x86_64-unknown-linux-gnu");
        assert_eq!(m[0].tag, "v0.1.0");
        assert_eq!(m[0].group, "default");
        assert_eq!(m[0].runner.as_deref(), Some("ubuntu-latest"));
        assert_eq!(m[1].runner.as_deref(), Some("macos-14"));
    }

    #[test]
    fn rows_get_per_group_tags() {
        let cfg = Config::from_toml(indoc! {r#"
            [[group]]
            name = "sdk"
            components = [
              { id = "py-sdk", type = "regex", path = "py/pyproject.toml",
                pattern = '(?P<version>[0-9.]+)',
                artifact = { kind = "python-wheel", path = "py" } },
            ]

            [[group]]
            name = "charts"
            components = [
              { id = "gateway", type = "helm-chart", path = "Chart.yaml",
                artifact = { kind = "helm-chart", chart = ".", registry = "oci://r" } },
            ]
        "#})
        .unwrap();
        let m = build_matrix(&cfg, &versions(&[("sdk", "0.4.1"), ("charts", "1.4.2")]));
        let py = m.iter().find(|r| r.name == "py-sdk").unwrap();
        let gw = m.iter().find(|r| r.name == "gateway").unwrap();
        assert_eq!(py.tag, "py-sdk/v0.4.1");
        assert_eq!(py.group, "sdk");
        assert_eq!(gw.tag, "gateway/v1.4.2");
        assert_eq!(gw.group, "charts");
    }

    #[test]
    fn signing_block_stamps_signable_rows() {
        let cfg = Config::from_toml(indoc! {r#"
            [signing]

            [[group]]
            name = "default"
            components = [
              { id = "api", type = "cargo-workspace", path = "Cargo.toml",
                artifact = { kind = "oci-image", context = ".", dockerfile = "Dockerfile",
                  registry = "ghcr.io/example/api" } },
            ]
        "#})
        .unwrap();
        let m = build_matrix(&cfg, &versions(&[("default", "0.1.0")]));
        assert_eq!(m[0].sign, Some(true));
        assert_eq!(
            m[0].fulcio_url.as_deref(),
            Some("https://fulcio.sigstore.dev")
        );
    }

    #[test]
    fn npm_rows_never_signed_even_when_enabled() {
        let cfg = Config::from_toml(indoc! {r#"
            [signing]

            [[group]]
            name = "default"
            components = [
              { id = "sdk", type = "package-json", path = "package.json",
                artifact = { kind = "npm-package", path = "ts/packages/sdk" } },
            ]
        "#})
        .unwrap();
        let m = build_matrix(&cfg, &versions(&[("default", "0.1.0")]));
        assert_eq!(m[0].sign, None);
        assert!(m[0].fulcio_url.is_none());
    }

    #[test]
    fn named_oci_registry_resolves_url_id_and_basic_auth() {
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
        let m = build_matrix(&cfg, &versions(&[("default", "1.2.3")]));
        let row = &m[0];
        // url + component id form the image repo; version is the image tag.
        assert_eq!(row.registry.as_deref(), Some("docker.io/tractorbeam/api"));
        assert_eq!(row.version, "1.2.3");
        assert_eq!(row.auth_kind, "basic");
        assert_eq!(row.username_secret.as_deref(), Some("DH_USER"));
        assert_eq!(row.password_secret.as_deref(), Some("DH_PAT"));
    }

    #[test]
    fn github_token_auth_threads_through() {
        let cfg = Config::from_toml(indoc! {r#"
            [registries.ghcr]
            kind = "oci"
            url = "ghcr.io/tractorbeamai"
            auth = { type = "github-token" }

            [[group]]
            name = "default"
            components = [
              { id = "api", type = "cargo-workspace", path = "Cargo.toml",
                artifact = { kind = "oci-image", context = ".", dockerfile = "Dockerfile",
                  registry = "ghcr" } },
            ]
        "#})
        .unwrap();
        let m = build_matrix(&cfg, &versions(&[("default", "0.1.0")]));
        assert_eq!(m[0].registry.as_deref(), Some("ghcr.io/tractorbeamai/api"));
        assert_eq!(m[0].auth_kind, "github-token");
        assert!(m[0].username_secret.is_none());
    }

    #[test]
    fn bare_url_registry_used_as_is_with_no_auth() {
        let cfg = Config::from_toml(indoc! {r#"
            [[group]]
            name = "default"
            components = [
              { id = "api", type = "cargo-workspace", path = "Cargo.toml",
                artifact = { kind = "oci-image", context = ".", dockerfile = "Dockerfile",
                  registry = "ghcr.io/example/api" } },
            ]
        "#})
        .unwrap();
        let m = build_matrix(&cfg, &versions(&[("default", "0.1.0")]));
        // No `/id` appended — the bare URL is the full repo.
        assert_eq!(m[0].registry.as_deref(), Some("ghcr.io/example/api"));
        assert_eq!(m[0].auth_kind, "none");
    }

    #[test]
    fn render_wraps_in_include() {
        let cfg = Config::from_toml(indoc! {r#"
            [[group]]
            name = "default"
            components = [
              { id = "platform", type = "helm-chart", path = "Chart.yaml",
                artifact = { kind = "helm-chart", chart = "deploy/helm/platform",
                  registry = "oci://ghcr.io/example/charts" } },
            ]
        "#})
        .unwrap();
        let m = build_matrix(&cfg, &versions(&[("default", "0.1.0")]));
        let v = render_for_actions(&m);
        assert_eq!(v["include"].as_array().unwrap().len(), 1);
        assert_eq!(v["include"][0]["kind"], "helm-chart");
        assert_eq!(v["include"][0]["tag"], "platform/v0.1.0");
    }
}
