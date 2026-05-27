//! Structured publish records — what a release actually shipped.
//!
//! Each artifact row in the release workflow emits one [`PublishRecord`] as
//! machine-readable JSON, rather than the workflow scraping a tool's
//! human-readable stdout (the brittle seam the `changesets/action` inherits
//! from regex-matching `changeset publish` output). The publish job then
//! merges the per-row records into one [`manifest`] attached to the release.
//!
//! Downstream consumers — GitHub Release bodies, notifications, and Phase D
//! attestation — read exact artifact identities and digests from these records
//! instead of re-deriving them.

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

/// One published artifact: its identity, the tag/version it shipped under, and
/// (for registry artifacts) the content digest the push resolved to.
///
/// Optional fields are omitted from the JSON when absent, so a record only
/// carries what its kind actually has — a `cli-binary` has a `target`/`sha256`
/// and no `digest`; an `oci-image` has a `registry`/`digest` and no `target`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublishRecord {
    /// Artifact kind: `oci-image`, `helm-chart`, `cli-binary`, `npm-package`,
    /// or `python-wheel`.
    pub kind: String,
    /// Component id — the artifact's public name.
    pub name: String,
    /// Release group the component belongs to.
    pub group: String,
    /// Git tag the artifact released under, e.g. `api/v0.5.3`.
    pub tag: String,
    /// Bare version, e.g. `0.5.3`.
    pub version: String,
    /// Registry/repository the artifact was published to, for kinds that have
    /// one (`oci-image`, `helm-chart`, `npm-package`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    /// Content digest (`sha256:…`) for registry artifacts (`oci-image`,
    /// `helm-chart`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    /// Target triple, for `cli-binary` rows (one record per target).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Tarball SHA-256 (hex), for `cli-binary` rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Release asset filename, when the artifact is a GitHub Release asset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
}

impl PublishRecord {
    /// Serialize to a single-line JSON object.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails (it shouldn't for this shape).
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self).context("serializing publish record")
    }
}

/// Merge per-row records into a stable-ordered manifest.
///
/// Sorted by `(group, name, target)` so the manifest is deterministic
/// regardless of the order matrix jobs finished in — diffs and downstream
/// consumers see a stable shape.
#[must_use]
pub fn manifest(mut records: Vec<PublishRecord>) -> Vec<PublishRecord> {
    records.sort_by(|a, b| {
        a.group
            .cmp(&b.group)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.target.cmp(&b.target))
    });
    records
}

/// Parse a slice of JSON record strings into a sorted manifest.
///
/// # Errors
///
/// Returns an error naming the offending input if any string isn't a valid
/// [`PublishRecord`].
pub fn manifest_from_json(records: &[String]) -> Result<Vec<PublishRecord>> {
    let parsed = records
        .iter()
        .enumerate()
        .map(|(i, s)| {
            serde_json::from_str::<PublishRecord>(s)
                .with_context(|| format!("parsing publish record #{i}"))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(manifest(parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oci(name: &str, group: &str) -> PublishRecord {
        PublishRecord {
            kind: "oci-image".into(),
            name: name.into(),
            group: group.into(),
            tag: format!("{name}/v1.0.0"),
            version: "1.0.0".into(),
            registry: Some(format!("ghcr.io/x/{name}")),
            digest: Some("sha256:abc".into()),
            target: None,
            sha256: None,
            asset: None,
        }
    }

    #[test]
    fn omits_absent_optional_fields() {
        let json = oci("api", "default").to_json().unwrap();
        assert!(json.contains("\"digest\":\"sha256:abc\""), "{json}");
        // cli-binary-only field *keys* aren't present on an oci-image record.
        // (Match quoted keys, not bare words — the digest value contains
        // "sha256".)
        assert!(!json.contains("\"target\""), "{json}");
        assert!(!json.contains("\"sha256\""), "{json}");
        assert!(!json.contains("\"asset\""), "{json}");
    }

    #[test]
    fn cli_binary_record_carries_target_and_sha() {
        let rec = PublishRecord {
            kind: "cli-binary".into(),
            name: "porter".into(),
            group: "default".into(),
            tag: "v1.0.0".into(),
            version: "1.0.0".into(),
            registry: None,
            digest: None,
            target: Some("x86_64-unknown-linux-gnu".into()),
            sha256: Some("deadbeef".into()),
            asset: Some("porter-x86_64-unknown-linux-gnu.tar.gz".into()),
        };
        let json = rec.to_json().unwrap();
        assert!(json.contains("x86_64-unknown-linux-gnu"), "{json}");
        assert!(!json.contains("registry"), "{json}");
        assert!(!json.contains("digest"), "{json}");
    }

    #[test]
    fn manifest_sorts_by_group_name_target() {
        let records = vec![oci("worker", "default"), oci("api", "default")];
        let m = manifest(records);
        assert_eq!(m[0].name, "api");
        assert_eq!(m[1].name, "worker");
    }

    #[test]
    fn manifest_from_json_roundtrips_and_sorts() {
        let inputs = vec![
            oci("web", "default").to_json().unwrap(),
            oci("api", "default").to_json().unwrap(),
        ];
        let m = manifest_from_json(&inputs).unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].name, "api");
    }

    #[test]
    fn manifest_from_json_reports_bad_input() {
        let err = manifest_from_json(&["not json".into()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("publish record #0"), "{err}");
    }
}
