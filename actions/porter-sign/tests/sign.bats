#!/usr/bin/env bats
# Unit tests for actions/porter-sign/sign.sh.
#
# `cosign`, `gh`, and `porter` are stubbed (under tests/stubs/) so the suite
# runs with no network, no real signing, and no porter install. Each stub
# appends its invocation to $STUB_LOG; the real `jq`/`mktemp` etc. stay on
# PATH. Tests assert the script signs the right ref, attaches the porter
# predicate, and emits + uploads the publish record per kind.

setup() {
  ACTION_DIR="$BATS_TEST_DIRNAME/.."
  SCRIPT="$ACTION_DIR/sign.sh"

  STUBS="$BATS_TEST_TMPDIR/stubs"
  mkdir -p "$STUBS"
  cp "$BATS_TEST_DIRNAME"/stubs/* "$STUBS/"
  chmod +x "$STUBS"/*
  export PATH="$STUBS:$PATH"

  export STUB_LOG="$BATS_TEST_TMPDIR/log"
  : > "$STUB_LOG"

  # dist/ is created relative to CWD; keep it in the test tmpdir.
  cd "$BATS_TEST_TMPDIR"

  # Record identity shared by most tests; kind-specific vars set per test.
  export NAME=api GROUP=default TAG=api/v0.5.3 VERSION=0.5.3 GH_TOKEN=fake
  export FULCIO_URL="" REKOR_URL="" REF="" REGISTRY="" ARTIFACT="" TARGET="" SHA256=""
}

# ---------- oci-image -------------------------------------------------

@test "oci-image: signs the digest ref, attests the predicate, records + uploads" {
  export KIND=oci-image SIGN=true ID=oci-api
  export REF="reg.example/api@sha256:abc" REGISTRY="reg.example/api"

  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  grep -q '^cosign sign --yes reg.example/api@sha256:abc$' "$STUB_LOG"
  grep -q '^porter attest --emit predicate --source-ref refs/tags/api/v0.5.3$' "$STUB_LOG"
  grep -q 'cosign attest --yes --predicate predicate.json --type slsaprovenance1 reg.example/api@sha256:abc' "$STUB_LOG"
  grep -q 'porter release record --kind oci-image .* --registry reg.example/api --digest sha256:abc' "$STUB_LOG"
  grep -q 'gh release upload api/v0.5.3 --clobber dist/published-oci-api.json' "$STUB_LOG"
  [ -f dist/published-oci-api.json ]
}

# ---------- helm-chart ------------------------------------------------

@test "helm-chart: signs the chart ref and records the base registry + digest" {
  export KIND=helm-chart SIGN=true ID=helm-foo NAME=foo
  export REF="reg.example/charts/foo@sha256:def" REGISTRY="oci://reg.example/charts"

  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  grep -q '^cosign sign --yes reg.example/charts/foo@sha256:def$' "$STUB_LOG"
  grep -q 'porter release record --kind helm-chart .* --registry oci://reg.example/charts --digest sha256:def' "$STUB_LOG"
  grep -q 'gh release upload api/v0.5.3 --clobber dist/published-helm-foo.json' "$STUB_LOG"
}

# ---------- cli-binary ------------------------------------------------

@test "cli-binary: sign-blob with sibling bundles, attest-blob, upload bundles + record" {
  export KIND=cli-binary SIGN=true ID=cli-x86 NAME=porter
  export ARTIFACT="dist/porter-x86_64-unknown-linux-gnu.tar.gz"
  export TARGET=x86_64-unknown-linux-gnu SHA256=deadbeef

  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  grep -q 'cosign sign-blob --yes --bundle dist/porter-x86_64-unknown-linux-gnu.sig.bundle dist/porter-x86_64-unknown-linux-gnu.tar.gz' "$STUB_LOG"
  grep -q 'cosign attest-blob --yes --predicate predicate.json --type slsaprovenance1 --bundle dist/porter-x86_64-unknown-linux-gnu.att.bundle dist/porter-x86_64-unknown-linux-gnu.tar.gz' "$STUB_LOG"
  grep -q 'gh release upload api/v0.5.3 --clobber dist/porter-x86_64-unknown-linux-gnu.sig.bundle dist/porter-x86_64-unknown-linux-gnu.att.bundle' "$STUB_LOG"
  grep -q 'porter release record --kind cli-binary .* --target x86_64-unknown-linux-gnu --sha256 deadbeef --asset porter-x86_64-unknown-linux-gnu.tar.gz' "$STUB_LOG"
}

# ---------- record-only (unsigned kinds) ------------------------------

@test "npm-package: sign=false records only, never invokes cosign" {
  export KIND=npm-package SIGN=false ID=npm-pkg NAME=pkg
  export REGISTRY="https://registry.npmjs.org"

  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  ! grep -q '^cosign' "$STUB_LOG"
  grep -q 'porter release record --kind npm-package .* --registry https://registry.npmjs.org' "$STUB_LOG"
  grep -q 'gh release upload api/v0.5.3 --clobber dist/published-npm-pkg.json' "$STUB_LOG"
}

@test "python-wheel: records common fields only, no registry/digest, no cosign" {
  export KIND=python-wheel SIGN=false ID=py-wheel NAME=wheel

  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  ! grep -q '^cosign' "$STUB_LOG"
  grep -q 'porter release record --kind python-wheel --name wheel --group default --tag api/v0.5.3 --version 0.5.3$' "$STUB_LOG"
}

# ---------- endpoint flags --------------------------------------------

@test "fulcio/rekor URLs are threaded onto cosign when set" {
  export KIND=oci-image SIGN=true ID=oci-api
  export REF="reg.example/api@sha256:abc" REGISTRY="reg.example/api"
  export FULCIO_URL="https://fulcio.test" REKOR_URL="https://rekor.test"

  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  grep -q 'cosign sign --yes --fulcio-url https://fulcio.test --rekor-url https://rekor.test reg.example/api@sha256:abc' "$STUB_LOG"
}

@test "no endpoint URLs ⇒ no --fulcio-url/--rekor-url flags (cosign defaults)" {
  export KIND=oci-image SIGN=true ID=oci-api
  export REF="reg.example/api@sha256:abc" REGISTRY="reg.example/api"

  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  grep -q '^cosign sign --yes reg.example/api@sha256:abc$' "$STUB_LOG"
  ! grep -q 'fulcio-url' "$STUB_LOG"
}

# ---------- validation ------------------------------------------------

@test "oci-image with sign=true but no ref errors clearly" {
  export KIND=oci-image SIGN=true ID=oci-api REGISTRY="reg.example/api"

  run bash "$SCRIPT"
  [ "$status" -ne 0 ]
  [[ "$output" == *"ref (<repo>@<digest>) is required"* ]]
}
