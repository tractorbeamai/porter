#!/usr/bin/env bats
# Unit tests for actions/porter-manifest/manifest.sh.
#
# `gh` and `porter` are stubbed (tests/stubs/); real `jq`/`cat`/`sort` stay on
# PATH. The stubbed Release asset list comes from $STUB_ASSETS. Asserts that
# each tag's per-row checksums + records are merged and re-uploaded, and the
# summary table is written.

setup() {
  ACTION_DIR="$BATS_TEST_DIRNAME/.."
  SCRIPT="$ACTION_DIR/manifest.sh"

  STUBS="$BATS_TEST_TMPDIR/stubs"
  mkdir -p "$STUBS"
  cp "$BATS_TEST_DIRNAME"/stubs/* "$STUBS/"
  chmod +x "$STUBS"/*
  export PATH="$STUBS:$PATH"

  export STUB_LOG="$BATS_TEST_TMPDIR/log"
  : > "$STUB_LOG"
  cd "$BATS_TEST_TMPDIR"

  export GITHUB_STEP_SUMMARY="$BATS_TEST_TMPDIR/summary"
  : > "$GITHUB_STEP_SUMMARY"
  export GH_TOKEN="fake"

  export TAGS="api/v0.5.3"
  export STUB_ASSETS=$'checksums-cli.txt\npublished-api.json'
}

@test "merges checksums + records, re-uploads both, writes the summary table" {
  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  grep -q 'gh release upload api/v0.5.3 --clobber .*/checksums.txt' "$STUB_LOG"
  grep -q 'porter release manifest' "$STUB_LOG"
  grep -q 'gh release upload api/v0.5.3 --clobber .*/published.json' "$STUB_LOG"
  grep -q '### api/v0.5.3' "$GITHUB_STEP_SUMMARY"
  grep -q '| oci-image | api | 0.5.3 | sha256:abc |' "$GITHUB_STEP_SUMMARY"
}

@test "a tag with no matching assets is skipped (no uploads, no summary)" {
  export STUB_ASSETS=""

  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  ! grep -q 'release upload' "$STUB_LOG"
  ! grep -q '###' "$GITHUB_STEP_SUMMARY"
}

@test "checksums-only release uploads checksums.txt but no manifest" {
  export STUB_ASSETS="checksums-cli.txt"

  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  grep -q 'gh release upload api/v0.5.3 --clobber .*/checksums.txt' "$STUB_LOG"
  ! grep -q 'published.json' "$STUB_LOG"
  ! grep -q 'porter release manifest' "$STUB_LOG"
}

@test "finalizes every tag in the list" {
  export TAGS=$'api/v0.5.3\nweb/v1.0.0'

  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  grep -q 'gh release upload api/v0.5.3 --clobber .*/published.json' "$STUB_LOG"
  grep -q 'gh release upload web/v1.0.0 --clobber .*/published.json' "$STUB_LOG"
}
