//! `porter attest` — emit in-toto v1 Statements with SLSA Build
//! Provenance v1 as the predicate.
//!
//! Phase D scaffolding: this module produces *unsigned* statements as
//! JSON. The signing layer (Sigstore keyless via Fulcio + Rekor) wraps
//! the statement in a DSSE envelope; that wrap happens in CI by piping
//! the output of `porter attest` through `cosign attest`. We deliberately
//! keep the pure-data part of attestation in Rust so it's testable and
//! reproducible from the same binary developers run locally.
//!
//! References:
//! - in-toto v1 Statement format:
//!   <https://github.com/in-toto/attestation/blob/main/spec/v1/statement.md>
//! - SLSA Provenance v1.0:
//!   <https://slsa.dev/spec/v1.0/provenance>

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub const STATEMENT_TYPE: &str = "https://in-toto.io/Statement/v1";
pub const PROVENANCE_PREDICATE_TYPE: &str = "https://slsa.dev/provenance/v1";
pub const BUILDER_ID: &str = "https://github.com/tractorbeamai/porter";

/// Top-level in-toto v1 Statement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Statement {
    #[serde(rename = "_type")]
    pub typ: String,
    pub subject: Vec<Subject>,
    #[serde(rename = "predicateType")]
    pub predicate_type: String,
    pub predicate: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Subject {
    pub name: String,
    pub digest: BTreeMap<String, String>,
}

/// SLSA Provenance v1.0 predicate. We use a structured type for the
/// fields porter knows about and pass everything through as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlsaProvenance {
    #[serde(rename = "buildDefinition")]
    pub build_definition: BuildDefinition,
    #[serde(rename = "runDetails")]
    pub run_details: RunDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildDefinition {
    #[serde(rename = "buildType")]
    pub build_type: String,
    #[serde(rename = "externalParameters")]
    pub external_parameters: serde_json::Value,
    #[serde(rename = "internalParameters", skip_serializing_if = "Option::is_none")]
    pub internal_parameters: Option<serde_json::Value>,
    #[serde(
        rename = "resolvedDependencies",
        skip_serializing_if = "Vec::is_empty",
        default
    )]
    pub resolved_dependencies: Vec<ResolvedDependency>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedDependency {
    pub uri: String,
    pub digest: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunDetails {
    pub builder: Builder,
    pub metadata: Metadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Builder {
    pub id: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub version: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    #[serde(rename = "invocationId")]
    pub invocation_id: String,
    #[serde(rename = "startedOn", skip_serializing_if = "Option::is_none")]
    pub started_on: Option<String>,
    #[serde(rename = "finishedOn", skip_serializing_if = "Option::is_none")]
    pub finished_on: Option<String>,
}

/// Inputs for [`build_statement`]. Most fields come from the GitHub
/// Actions environment when running in CI; pass them in directly so the
/// function stays testable.
#[derive(Debug, Clone)]
pub struct AttestInput {
    pub subject_name: String,
    pub subject_sha256: String,
    pub source_repo: String,
    pub source_ref: String,
    pub source_sha: String,
    pub run_id: String,
    pub run_attempt: Option<String>,
    pub workflow_ref: Option<String>,
    pub started_on: Option<String>,
    pub finished_on: Option<String>,
    pub porter_version: String,
}

/// Build an in-toto v1 Statement with SLSA Build Provenance v1
/// embedded as the predicate.
///
/// # Errors
///
/// Returns an error if `input.source_repo` is not in a recognized form
/// (`owner/repo` short form or a full `https://...` URL), or if the
/// constructed provenance value fails to serialize. The latter is not
/// expected to happen — `SlsaProvenance` has no non-string map keys or
/// non-finite floats — but is propagated rather than panicked-on so
/// callers stay in `Result` land.
pub fn build_statement(input: &AttestInput) -> Result<Statement> {
    let mut digest = BTreeMap::new();
    digest.insert("sha256".to_owned(), input.subject_sha256.clone());

    let subject = Subject {
        name: input.subject_name.clone(),
        digest,
    };

    let repo_url = normalize_repo_url(&input.source_repo)?;

    let invocation_id = format!(
        "{repo_url}/actions/runs/{run_id}{attempt}",
        run_id = input.run_id,
        attempt = input
            .run_attempt
            .as_deref()
            .map(|a| format!("/attempts/{a}"))
            .unwrap_or_default()
    );

    let mut external = serde_json::Map::new();
    external.insert(
        "source".into(),
        serde_json::Value::String(format!("git+{repo_url}@{}", input.source_ref)),
    );
    if let Some(wf) = &input.workflow_ref {
        external.insert("workflow".into(), serde_json::Value::String(wf.clone()));
    }

    let provenance = SlsaProvenance {
        build_definition: BuildDefinition {
            build_type: "https://github.com/tractorbeamai/porter/build-types/release/v1".into(),
            external_parameters: serde_json::Value::Object(external),
            internal_parameters: None,
            resolved_dependencies: vec![ResolvedDependency {
                uri: format!("git+{repo_url}@{}", input.source_ref),
                digest: {
                    let mut m = BTreeMap::new();
                    m.insert("gitCommit".into(), input.source_sha.clone());
                    m
                },
            }],
        },
        run_details: RunDetails {
            builder: Builder {
                id: BUILDER_ID.into(),
                version: {
                    let mut m = BTreeMap::new();
                    m.insert("porter".into(), input.porter_version.clone());
                    m
                },
            },
            metadata: Metadata {
                invocation_id,
                started_on: input.started_on.clone(),
                finished_on: input.finished_on.clone(),
            },
        },
    };

    let predicate = serde_json::to_value(&provenance).context("serializing SLSA provenance")?;

    Ok(Statement {
        typ: STATEMENT_TYPE.into(),
        subject: vec![subject],
        predicate_type: PROVENANCE_PREDICATE_TYPE.into(),
        predicate,
    })
}

/// Compute SHA-256 of a file as a lowercase hex string.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or read.
pub fn sha256_hex(path: &Path) -> Result<String> {
    use std::io::Read as _;
    let mut f = std::fs::File::open(path)
        .with_context(|| format!("opening {} for hashing", path.display()))?;
    let mut hasher = Sha256::new();
    // Boxed to avoid a 64 KiB stack frame; the hot path of CI release
    // builds runs this on small runners with constrained stacks.
    let mut buf = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// GitHub repo strings come in two forms in the Actions environment:
/// `owner/repo` (from `GITHUB_REPOSITORY`) and the full URL. Always
/// normalize to a fully-qualified HTTPS URL with no trailing slash.
///
/// Inputs we don't recognize (`http://`, `git@host:org/repo.git`, etc.)
/// would silently produce malformed output if we tried to coerce them,
/// so reject explicitly.
fn normalize_repo_url(s: &str) -> Result<String> {
    if let Some(rest) = s.strip_prefix("git+https://") {
        Ok(format!("https://{}", rest.trim_end_matches('/')))
    } else if s.starts_with("https://") {
        Ok(s.trim_end_matches('/').to_owned())
    } else if !s.contains("://") && !s.contains('@') {
        let trimmed = s.trim_matches('/');
        if !trimmed.contains('/') {
            bail!("source repo {s:?} must be `owner/repo` or a full https:// URL");
        }
        Ok(format!("https://github.com/{trimmed}"))
    } else {
        bail!(
            "source repo {s:?} is not a recognized form (expected `owner/repo` or `https://...`)"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> AttestInput {
        AttestInput {
            subject_name: "porter-x86_64-unknown-linux-gnu.tar.gz".into(),
            subject_sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
                .into(),
            source_repo: "tractorbeamai/porter".into(),
            source_ref: "refs/tags/v0.1.0".into(),
            source_sha: "deadbeef0000000000000000000000000000beef".into(),
            run_id: "12345".into(),
            run_attempt: Some("1".into()),
            workflow_ref: Some(
                "tractorbeamai/porter/.github/workflows/porter-release.yml@refs/tags/v0.1.0".into(),
            ),
            started_on: Some("2026-05-07T22:00:00Z".into()),
            finished_on: Some("2026-05-07T22:05:00Z".into()),
            porter_version: "0.1.0".into(),
        }
    }

    #[test]
    fn statement_has_correct_top_level_shape() {
        let s = build_statement(&fixture()).unwrap();
        assert_eq!(s.typ, STATEMENT_TYPE);
        assert_eq!(s.predicate_type, PROVENANCE_PREDICATE_TYPE);
        assert_eq!(s.subject.len(), 1);
        assert_eq!(
            &s.subject[0].digest["sha256"],
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn invocation_id_includes_run_attempt() {
        let s = build_statement(&fixture()).unwrap();
        let metadata = &s.predicate["runDetails"]["metadata"]["invocationId"];
        assert_eq!(
            metadata,
            "https://github.com/tractorbeamai/porter/actions/runs/12345/attempts/1"
        );
    }

    #[test]
    fn external_parameters_carry_source_uri() {
        let s = build_statement(&fixture()).unwrap();
        let src = &s.predicate["buildDefinition"]["externalParameters"]["source"];
        assert_eq!(
            src,
            "git+https://github.com/tractorbeamai/porter@refs/tags/v0.1.0"
        );
    }

    #[test]
    fn resolved_dependencies_carry_git_commit() {
        let s = build_statement(&fixture()).unwrap();
        let dep0 = &s.predicate["buildDefinition"]["resolvedDependencies"][0];
        assert_eq!(
            dep0["digest"]["gitCommit"],
            "deadbeef0000000000000000000000000000beef"
        );
    }

    #[test]
    fn builder_id_is_pinned() {
        let s = build_statement(&fixture()).unwrap();
        assert_eq!(s.predicate["runDetails"]["builder"]["id"], BUILDER_ID);
    }

    #[test]
    fn normalize_repo_url_handles_short_form() {
        assert_eq!(
            normalize_repo_url("tractorbeamai/porter").unwrap(),
            "https://github.com/tractorbeamai/porter"
        );
    }

    #[test]
    fn normalize_repo_url_handles_full_url_with_trailing_slash() {
        assert_eq!(
            normalize_repo_url("https://github.com/tractorbeamai/porter/").unwrap(),
            "https://github.com/tractorbeamai/porter"
        );
    }

    #[test]
    fn normalize_repo_url_handles_git_plus_https() {
        assert_eq!(
            normalize_repo_url("git+https://github.com/tractorbeamai/porter").unwrap(),
            "https://github.com/tractorbeamai/porter"
        );
    }

    #[test]
    fn normalize_repo_url_bails_on_http_scheme() {
        let err = normalize_repo_url("http://github.com/tractorbeamai/porter")
            .unwrap_err()
            .to_string();
        assert!(err.contains("not a recognized form"), "got: {err}");
    }

    #[test]
    fn normalize_repo_url_bails_on_ssh_form() {
        let err = normalize_repo_url("git@github.com:tractorbeamai/porter.git")
            .unwrap_err()
            .to_string();
        assert!(err.contains("not a recognized form"), "got: {err}");
    }

    #[test]
    fn normalize_repo_url_bails_on_short_form_without_slash() {
        let err = normalize_repo_url("just-a-name").unwrap_err().to_string();
        assert!(err.contains("owner/repo"), "got: {err}");
    }

    #[test]
    fn build_statement_bails_on_unrecognized_repo_form() {
        let mut input = fixture();
        input.source_repo = "git@github.com:tractorbeamai/porter.git".into();
        let err = build_statement(&input).unwrap_err().to_string();
        assert!(err.contains("not a recognized form"), "got: {err}");
    }

    #[test]
    fn statement_roundtrips_through_json() {
        let s = build_statement(&fixture()).unwrap();
        let blob = serde_json::to_string(&s).unwrap();
        let s2: Statement = serde_json::from_str(&blob).unwrap();
        assert_eq!(s, s2);
    }

    #[test]
    fn sha256_hex_matches_known_vector() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("x");
        std::fs::write(&p, b"hello").unwrap();
        assert_eq!(
            sha256_hex(&p).unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}
