// Integration test for the bash glue in .github/workflows/version.yml.
//
// The version.yml reusable workflow does:
//
//   status_json=$(porter status --json)
//   next=$(echo "$status_json" | jq -r '.next // empty')
//   if [[ -z "$next" || "$next" == "null" ]]; then
//     echo "skip"
//   fi
//
// These tests pin the contract that `porter status --json` produces
// JSON whose `.next` field can be extracted by `jq -r '.next //
// empty'` and consumed by the `[[ -z "$next" ]]` skip check. If
// porter's status JSON ever drifts in a way that breaks this idiom,
// these tests fail before the reusable workflow does.

// Integration tests live in their own crate and don't inherit the
// `cfg_attr(test, ...)` allows from the binary.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic_in_result_fn,
    clippy::str_to_string,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::tests_outside_test_module
)]

use std::fs;
use std::io::Write as _;
use std::process::{Command, Stdio};

use indoc::indoc;
use tempfile::TempDir;

fn fixture(with_changeset: bool) -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    fs::write(
        root.join("porter.toml"),
        indoc! {r#"
            [changesets]
            directory = ".changeset"

            [[versioned_files]]
            type = "cargo-workspace"
            path = "Cargo.toml"

            [release]
            tag_prefix = "v"
            changelog = "CHANGELOG.md"
        "#},
    )
    .unwrap();
    fs::write(
        root.join("Cargo.toml"),
        indoc! {r#"
            [workspace]
            members = []

            [workspace.package]
            version = "0.1.0"
        "#},
    )
    .unwrap();
    fs::create_dir_all(root.join(".changeset")).unwrap();
    if with_changeset {
        fs::write(
            root.join(".changeset/a.md"),
            indoc! {"
                ---
                bump: minor
                ---

                Add a thing.
            "},
        )
        .unwrap();
    }
    dir
}

fn jq_or_skip() -> Option<String> {
    let probe = Command::new("jq").arg("--version").output().ok();
    match probe {
        Some(out) if out.status.success() => Some("jq".to_string()),
        _ => {
            eprintln!("skipping: jq not on PATH");
            None
        }
    }
}

fn status_json(dir: &TempDir) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_porter"))
        .args(["status", "--json"])
        .current_dir(dir.path())
        .output()
        .expect("running porter status --json");
    assert!(
        output.status.success(),
        "porter status --json failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn pipe_through_jq(jq: &str, json: &str, filter: &str) -> String {
    let mut child = Command::new(jq)
        .args(["-r", filter])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawning jq");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(json.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("waiting on jq");
    assert!(
        out.status.success(),
        "jq failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
fn jq_extracts_next_when_changesets_present() {
    let Some(jq) = jq_or_skip() else { return };
    let dir = fixture(true);
    let json = status_json(&dir);
    let next = pipe_through_jq(&jq, &json, ".next // empty");
    assert!(
        !next.is_empty() && next != "null",
        "expected a non-empty, non-null next; got {next:?}"
    );
    // Confirms the schema contract version.yml relies on:
    // a non-empty `.next` is a valid semver-looking string.
    assert!(
        next.split('.').count() == 3,
        "expected dotted-triple semver, got {next:?}"
    );
}

#[test]
fn jq_yields_empty_when_no_changesets() {
    let Some(jq) = jq_or_skip() else { return };
    let dir = fixture(false);
    let json = status_json(&dir);
    let next = pipe_through_jq(&jq, &json, ".next // empty");
    // version.yml then runs `[[ -z "$next" || "$next" == "null" ]]`
    // to decide whether to skip; both empty-string and "null" trigger
    // the skip path.
    assert!(
        next.is_empty(),
        "expected `.next // empty` to coalesce a null/missing field to '', got {next:?}"
    );
}

#[test]
fn bump_field_matches_jq_extraction() {
    // Pins that the `.bump` field, also read by some consumers,
    // is one of {patch, minor, major} when present.
    let Some(jq) = jq_or_skip() else { return };
    let dir = fixture(true);
    let json = status_json(&dir);
    let bump = pipe_through_jq(&jq, &json, ".bump // empty");
    assert!(
        matches!(bump.as_str(), "patch" | "minor" | "major"),
        "unexpected bump string: {bump:?}"
    );
}
