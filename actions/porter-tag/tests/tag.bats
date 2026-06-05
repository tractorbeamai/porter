#!/usr/bin/env bats
# Unit tests for actions/porter-tag/tag.sh.
#
# `porter`, `git`, and `gh` are stubbed (tests/stubs/); real `jq` stays on
# PATH. The stubbed matrix + tag list come from env so each test shapes the
# release. Asserts which tags are cut/pushed, the emitted outputs, and the
# per-(tag,group) Release creation — all idempotent on re-run.

setup() {
  ACTION_DIR="$BATS_TEST_DIRNAME/.."
  SCRIPT="$ACTION_DIR/tag.sh"

  STUBS="$BATS_TEST_TMPDIR/stubs"
  mkdir -p "$STUBS"
  cp "$BATS_TEST_DIRNAME"/stubs/* "$STUBS/"
  chmod +x "$STUBS"/*
  export PATH="$STUBS:$PATH"

  export STUB_LOG="$BATS_TEST_TMPDIR/log"
  : > "$STUB_LOG"
  cd "$BATS_TEST_TMPDIR"

  export GITHUB_OUTPUT="$BATS_TEST_TMPDIR/output"
  : > "$GITHUB_OUTPUT"
  export GITHUB_REPOSITORY="acme/widgets" GH_TOKEN="fake"
  export STUB_EXISTING_TAGS="" STUB_EXISTING_RELEASES=""

  # Two-component release, both in the default group.
  export STUB_MATRIX='{"include":[{"tag":"api/v0.5.3","group":"default"},{"tag":"worker/v0.5.3","group":"default"}]}'
  export STUB_TAGS=$'api/v0.5.3\nworker/v0.5.3'
}

@test "cuts + pushes all-new tags, emits outputs, creates a Release per tag" {
  run bash "$SCRIPT"
  [ "$status" -eq 0 ]

  grep -q 'git tag -a api/v0.5.3' "$STUB_LOG"
  grep -q 'git tag -a worker/v0.5.3' "$STUB_LOG"
  grep -q 'git push .* refs/tags/api/v0.5.3 refs/tags/worker/v0.5.3' "$STUB_LOG"

  grep -q '^has-artifacts=true$' "$GITHUB_OUTPUT"
  grep -q '^matrix=' "$GITHUB_OUTPUT"
  grep -q '^api/v0.5.3$' "$GITHUB_OUTPUT"
  grep -q '^worker/v0.5.3$' "$GITHUB_OUTPUT"

  grep -q 'gh release create api/v0.5.3' "$STUB_LOG"
  grep -q 'gh release create worker/v0.5.3' "$STUB_LOG"
}

@test "skips a tag that already exists; pushes only the new one" {
  export STUB_EXISTING_TAGS="api/v0.5.3"

  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  [[ "$output" == *"tag api/v0.5.3 already exists"* ]]
  ! grep -q 'git tag -a api/v0.5.3' "$STUB_LOG"
  grep -q 'git tag -a worker/v0.5.3' "$STUB_LOG"
  grep -q 'git push .* refs/tags/worker/v0.5.3' "$STUB_LOG"
  ! grep -q 'git push .*refs/tags/api/v0.5.3' "$STUB_LOG"
}

@test "no push when every tag already exists" {
  export STUB_EXISTING_TAGS="api/v0.5.3 worker/v0.5.3"

  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  ! grep -q 'git push' "$STUB_LOG"
}

@test "has-artifacts=false on an empty matrix" {
  export STUB_MATRIX='{"include":[]}' STUB_TAGS=""

  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  grep -q '^has-artifacts=false$' "$GITHUB_OUTPUT"
  ! grep -q 'git push' "$STUB_LOG"
}

@test "skips creating a Release that already exists" {
  export STUB_EXISTING_RELEASES="api/v0.5.3"

  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  [[ "$output" == *"release api/v0.5.3 already exists"* ]]
  ! grep -q 'gh release create api/v0.5.3' "$STUB_LOG"
  grep -q 'gh release create worker/v0.5.3' "$STUB_LOG"
}
