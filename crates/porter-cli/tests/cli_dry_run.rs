// Integration tests live in their own crates and don't pick up the
// `cfg_attr(test, ...)` allows from the binary, so the workspace's
// restriction-group lints have to be relaxed here too.
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
use std::process::Command;

use indoc::indoc;
use tempfile::TempDir;

fn fixture() -> TempDir {
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
    let cs_dir = root.join(".changeset");
    fs::create_dir_all(&cs_dir).unwrap();
    fs::write(
        cs_dir.join("a.md"),
        indoc! {"
            ---
            bump: minor
            ---

            Add a thing.
        "},
    )
    .unwrap();
    dir
}

#[test]
fn version_dry_run_does_not_modify_files() {
    let dir = fixture();
    let cargo_toml_before = fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_porter"))
        .args(["version", "--dry-run"])
        .current_dir(dir.path())
        .output()
        .expect("running porter");

    assert!(
        output.status.success(),
        "porter version --dry-run failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("would bump default: 0.1.0 ->"),
        "expected next-version line in stdout; got: {stdout}"
    );
    assert!(
        stdout.contains("would consume"),
        "expected dry-run consume preview; got: {stdout}"
    );

    let cargo_toml_after = fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
    assert_eq!(
        cargo_toml_before, cargo_toml_after,
        "Cargo.toml must not be modified by --dry-run"
    );
    assert!(
        dir.path().join(".changeset/a.md").exists(),
        "changeset must not be consumed by --dry-run"
    );
    assert!(
        !dir.path().join("CHANGELOG.md").exists(),
        "CHANGELOG.md must not be written by --dry-run"
    );
}

#[test]
fn status_json_emits_next_field() {
    let dir = fixture();

    let output = Command::new(env!("CARGO_BIN_EXE_porter"))
        .args(["status", "--json"])
        .current_dir(dir.path())
        .output()
        .expect("running porter");

    assert!(
        output.status.success(),
        "porter status --json failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("status JSON parses");
    let group = &parsed["groups"][0];
    assert_eq!(group["name"], "default");
    assert_eq!(group["current"], "0.1.0");
    assert!(
        group["next"].is_string(),
        "expected next to be a string; got {}",
        group["next"]
    );
    assert_eq!(group["bump"], "minor");
    assert_eq!(parsed["pr_title"], "Version Packages: 0.1.1");
}

#[test]
fn release_record_emits_structured_json() {
    let dir = fixture();

    let output = Command::new(env!("CARGO_BIN_EXE_porter"))
        .args([
            "release",
            "record",
            "--kind",
            "oci-image",
            "--name",
            "api",
            "--group",
            "default",
            "--tag",
            "api/v0.1.1",
            "--version",
            "0.1.1",
            "--registry",
            "ghcr.io/x/api",
            "--digest",
            "sha256:abc",
            // Empty values (the workflow passes these for kinds that lack
            // them) must be dropped, not serialized as "".
            "--target",
            "",
        ])
        .current_dir(dir.path())
        .output()
        .expect("running porter");

    assert!(
        output.status.success(),
        "porter release record failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rec: serde_json::Value =
        serde_json::from_str(&String::from_utf8(output.stdout).unwrap()).expect("record parses");
    assert_eq!(rec["kind"], "oci-image");
    assert_eq!(rec["name"], "api");
    assert_eq!(rec["digest"], "sha256:abc");
    assert!(
        rec.get("target").is_none(),
        "empty --target must be dropped"
    );
    assert!(rec.get("sha256").is_none());
}

#[test]
fn release_manifest_merges_and_sorts_records() {
    let dir = fixture();
    let root = dir.path();
    // Two records, intentionally out of sorted order.
    fs::write(
        root.join("published-b.json"),
        r#"{"kind":"oci-image","name":"worker","group":"default","tag":"worker/v0.1.1","version":"0.1.1"}"#,
    )
    .unwrap();
    fs::write(
        root.join("published-a.json"),
        r#"{"kind":"oci-image","name":"api","group":"default","tag":"api/v0.1.1","version":"0.1.1"}"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_porter"))
        .args([
            "release",
            "manifest",
            "published-b.json",
            "published-a.json",
        ])
        .current_dir(root)
        .output()
        .expect("running porter");

    assert!(
        output.status.success(),
        "porter release manifest failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest: serde_json::Value =
        serde_json::from_str(&String::from_utf8(output.stdout).unwrap()).expect("manifest parses");
    let arr = manifest.as_array().expect("manifest is an array");
    assert_eq!(arr.len(), 2);
    // Sorted by (group, name): api before worker.
    assert_eq!(arr[0]["name"], "api");
    assert_eq!(arr[1]["name"], "worker");
}
