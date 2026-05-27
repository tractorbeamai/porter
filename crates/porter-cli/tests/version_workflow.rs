// Integration test for the bash glue in .github/workflows/version.yml.
//
// The version.yml reusable workflow does:
//
//   status_json=$(porter status --json)
//   pr_title=$(echo "$status_json" | jq -r '.pr_title // empty')
//   if [[ -z "$pr_title" || "$pr_title" == "null" ]]; then
//     echo "skip"
//   fi
//
// With groups, there's no single top-level `.next` (each group has its own
// under `.groups[]`), so `.pr_title` is the release/skip signal: it's null
// exactly when no group has pending changesets. These tests pin that contract
// plus the per-group `.groups[].next`/`.bump` shape.

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

            [[group]]
            name = "default"
            components = [
              { id = "porter", type = "cargo-workspace", path = "Cargo.toml", tag_prefix = "v" },
            ]
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
    let next = pipe_through_jq(&jq, &json, ".groups[0].next // empty");
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
    let next = pipe_through_jq(&jq, &json, ".groups[0].next // empty");
    // With no changesets a group's `.next` is null; `// empty` coalesces it
    // to '' so the per-group view reads cleanly.
    assert!(
        next.is_empty(),
        "expected `.groups[0].next // empty` to coalesce null to '', got {next:?}"
    );
}

#[test]
fn bump_field_matches_jq_extraction() {
    // Pins that the `.bump` field, also read by some consumers,
    // is one of {patch, minor, major} when present.
    let Some(jq) = jq_or_skip() else { return };
    let dir = fixture(true);
    let json = status_json(&dir);
    let bump = pipe_through_jq(&jq, &json, ".groups[0].bump // empty");
    assert!(
        matches!(bump.as_str(), "patch" | "minor" | "major"),
        "unexpected bump string: {bump:?}"
    );
}

#[test]
fn pr_title_rendered_from_default_template() {
    // version.yml reads `.pr_title` for the PR title / commit subject.
    // With the default config it's "Version Packages: <next>".
    let Some(jq) = jq_or_skip() else { return };
    let dir = fixture(true);
    let json = status_json(&dir);
    let next = pipe_through_jq(&jq, &json, ".groups[0].next // empty");
    let pr_title = pipe_through_jq(&jq, &json, ".pr_title // empty");
    assert_eq!(pr_title, format!("Version Packages: {next}"));
}

#[test]
fn pr_title_empty_when_no_changesets() {
    // The skip path: `.pr_title // empty` must coalesce to '' like `.next`.
    let Some(jq) = jq_or_skip() else { return };
    let dir = fixture(false);
    let json = status_json(&dir);
    let pr_title = pipe_through_jq(&jq, &json, ".pr_title // empty");
    assert!(
        pr_title.is_empty(),
        "expected empty pr_title with no changesets, got {pr_title:?}"
    );
}
