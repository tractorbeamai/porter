#!/usr/bin/env bats
# Unit tests for actions/setup-porter/setup.sh.
#
# These run against a stubbed `curl` (under tests/stubs/) so no network
# calls happen and no real porter release is required. The tests
# regenerate fresh fixtures per test (tarball + matching checksum) so a
# `setup.sh install` invocation completes end-to-end up to and including
# the post-install `--version` smoke run.

setup() {
  ACTION_DIR="$BATS_TEST_DIRNAME/.."
  SCRIPT="$ACTION_DIR/setup.sh"

  WORK="$BATS_TEST_TMPDIR/work"
  FIX="$BATS_TEST_TMPDIR/fix"
  STUBS="$BATS_TEST_TMPDIR/stubs"
  INSTALL_DIR="$BATS_TEST_TMPDIR/install"
  mkdir -p "$WORK" "$FIX" "$STUBS" "$INSTALL_DIR"

  # Wire up the curl stub. The real `jq`, `tar`, `shasum`, and `grep`
  # stay on PATH — they're cheap and we want to exercise them for real.
  cp "$BATS_TEST_DIRNAME/stubs/curl" "$STUBS/curl"
  chmod +x "$STUBS/curl"
  export STUB_FIXTURES="$FIX"
  export PATH="$STUBS:$PATH"

  # action.yml passes outputs/path via these env vars; create empty
  # files so `>>` works.
  export GITHUB_OUTPUT="$WORK/output"
  export GITHUB_PATH="$WORK/path"
  : > "$GITHUB_OUTPUT"
  : > "$GITHUB_PATH"
}

# Build a tarball that contains an executable `porter` shell stub and
# write the matching BSD-format checksum line into checksums.txt.
make_fixture_release() {
  local target="$1"
  local asset="porter-${target}.tar.gz"

  # Create a fake porter binary that prints a known version string when
  # invoked with --version (matches what the action does post-install).
  cat > "$FIX/porter" <<'EOF'
#!/usr/bin/env bash
[[ "$1" == "--version" ]] && echo "porter 0.6.0" && exit 0
echo "fake porter" && exit 0
EOF
  chmod +x "$FIX/porter"
  (cd "$FIX" && tar -czf "$asset" porter && rm porter)

  local sha
  sha=$(shasum -a 256 "$FIX/$asset" | awk '{print $1}')
  printf '%s  %s\n' "$sha" "$asset" > "$FIX/checksums.txt"
}

# ---------- resolve ---------------------------------------------------

@test "resolve: passes a pinned version through unchanged" {
  export REQ_VERSION=v0.5.0
  export GH_TOKEN=fake

  run bash "$SCRIPT" resolve
  [ "$status" -eq 0 ]
  grep -q '^version=v0.5.0$' "$GITHUB_OUTPUT"
}

@test "resolve: latest resolves via the releases API" {
  printf '%s\n' '{"tag_name":"v0.6.0"}' > "$FIX/latest.json"
  export REQ_VERSION=latest
  export GH_TOKEN=fake

  run bash "$SCRIPT" resolve
  [ "$status" -eq 0 ]
  grep -q '^version=v0.6.0$' "$GITHUB_OUTPUT"
}

@test "resolve: latest emits the warn-on-floating annotation" {
  printf '%s\n' '{"tag_name":"v0.6.0"}' > "$FIX/latest.json"
  export REQ_VERSION=latest
  export GH_TOKEN=fake

  run bash "$SCRIPT" resolve
  [ "$status" -eq 0 ]
  [[ "$output" == *"::warning::setup-porter:"* ]]
}

@test "resolve: errors when the API returns null tag_name" {
  printf '%s\n' '{"tag_name":null}' > "$FIX/latest.json"
  export REQ_VERSION=latest
  export GH_TOKEN=fake

  run bash "$SCRIPT" resolve
  [ "$status" -ne 0 ]
  [[ "$output" == *"could not resolve porter version"* ]]
}

# ---------- install ---------------------------------------------------

@test "install: linux x86_64 picks the gnu target" {
  make_fixture_release x86_64-unknown-linux-gnu
  export RUNNER_OS=Linux RUNNER_ARCH=X64
  export VERSION=v0.6.0 INSTALL_DIR="$INSTALL_DIR"

  run bash "$SCRIPT" install
  [ "$status" -eq 0 ]
  [[ "$output" == *"porter 0.6.0"* ]]
  grep -q "^path=$INSTALL_DIR/porter$" "$GITHUB_OUTPUT"
  grep -q "^$INSTALL_DIR$" "$GITHUB_PATH"
}

@test "install: macOS arm64 picks the aarch64-darwin target" {
  make_fixture_release aarch64-apple-darwin
  export RUNNER_OS=macOS RUNNER_ARCH=ARM64
  export VERSION=v0.6.0 INSTALL_DIR="$INSTALL_DIR"

  run bash "$SCRIPT" install
  [ "$status" -eq 0 ]
  [[ "$output" == *"porter 0.6.0"* ]]
}

@test "install: rejects an unsupported runner with a clear error" {
  export RUNNER_OS=Windows RUNNER_ARCH=X64
  export VERSION=v0.6.0 INSTALL_DIR="$INSTALL_DIR"

  run bash "$SCRIPT" install
  [ "$status" -ne 0 ]
  [[ "$output" == *"unsupported runner Windows-X64"* ]]
}

@test "install: rejects a tampered checksum (regression for two-space anchor)" {
  make_fixture_release x86_64-unknown-linux-gnu
  # Flip the sha to something definitely wrong.
  sed -i.bak 's/^[a-f0-9]\{64\}/0000000000000000000000000000000000000000000000000000000000000000/' "$FIX/checksums.txt"
  export RUNNER_OS=Linux RUNNER_ARCH=X64
  export VERSION=v0.6.0 INSTALL_DIR="$INSTALL_DIR"

  run bash "$SCRIPT" install
  [ "$status" -ne 0 ]
}

@test "install: rejects when the asset name doesn't appear in checksums.txt" {
  make_fixture_release x86_64-unknown-linux-gnu
  # Replace the asset name with something else so the grep finds nothing.
  sed -i.bak 's/porter-x86_64-unknown-linux-gnu/porter-other-target/' "$FIX/checksums.txt"
  export RUNNER_OS=Linux RUNNER_ARCH=X64
  export VERSION=v0.6.0 INSTALL_DIR="$INSTALL_DIR"

  run bash "$SCRIPT" install
  [ "$status" -ne 0 ]
}

# ---------- argument validation ---------------------------------------

@test "errors on an unknown verb" {
  run bash "$SCRIPT" frobnicate
  [ "$status" -eq 2 ]
  [[ "$output" == *"unknown verb 'frobnicate'"* ]]
}

@test "errors when invoked without a verb" {
  run bash "$SCRIPT"
  [ "$status" -eq 2 ]
}
