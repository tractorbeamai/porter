#!/usr/bin/env bash
# The body of the `setup-porter` composite action, factored out of
# action.yml so it can be unit-tested with bats and statically
# checked with shellcheck. action.yml invokes this script once per
# step (`resolve` then `install`), passing inputs via env vars.
#
# Inputs (env vars), per verb:
#
#   resolve:
#     GH_TOKEN        — token for the GitHub Releases API call.
#     REQ_VERSION     — the user's `version:` input. `latest` triggers
#                       a runtime API resolution; anything else is
#                       passed through verbatim.
#     GITHUB_OUTPUT   — file the GH Actions runner reads for outputs.
#
#   install:
#     GH_TOKEN        — currently unused on the download path (the
#                       release artifacts are public) but kept in the
#                       env contract for symmetry with `resolve`.
#     VERSION         — the resolved version (e.g. `v0.6.0`).
#     INSTALL_DIR     — directory the porter binary is installed into.
#     RUNNER_OS       — `Linux` / `macOS` / `Windows` (GH-provided).
#     RUNNER_ARCH     — `X64` / `ARM64` / etc. (GH-provided).
#     GITHUB_OUTPUT   — file the runner reads for outputs.
#     GITHUB_PATH     — file the runner appends to PATH.

set -euo pipefail

verb="${1:-}"
case "$verb" in
  resolve) ;;
  install) ;;
  *)
    echo "::error::setup-porter: unknown verb '$verb' (expected: resolve, install)" >&2
    exit 2
    ;;
esac

if [[ "$verb" == "resolve" ]]; then
  if [[ "$REQ_VERSION" == "latest" ]]; then
    echo "::warning::setup-porter: pinning to a specific vX.Y.Z tag is strongly recommended; \`latest\` resolves at runtime and floats." >&2
    # Resolve `latest` via the GitHub Releases API rather than the
    # /releases/latest endpoint, since that endpoint hides prereleases
    # and we want explicit control later.
    version=$(curl -fsSL \
      -H "Authorization: Bearer $GH_TOKEN" \
      -H "Accept: application/vnd.github+json" \
      "https://api.github.com/repos/tractorbeamai/porter/releases/latest" \
      | jq -r .tag_name)
  else
    version="$REQ_VERSION"
  fi
  if [[ -z "$version" || "$version" == "null" ]]; then
    echo "::error::could not resolve porter version" >&2
    exit 1
  fi
  echo "version=$version" >> "$GITHUB_OUTPUT"
  exit 0
fi

# verb == install

case "$RUNNER_OS-$RUNNER_ARCH" in
  Linux-X64)   target="x86_64-unknown-linux-gnu" ;;
  Linux-ARM64) target="aarch64-unknown-linux-gnu" ;;
  macOS-X64)   target="x86_64-apple-darwin" ;;
  macOS-ARM64) target="aarch64-apple-darwin" ;;
  *) echo "::error::unsupported runner $RUNNER_OS-$RUNNER_ARCH" >&2; exit 1 ;;
esac

asset="porter-${target}.tar.gz"
# shellcheck disable=SC2153  # VERSION is an action input, not a typo of the resolve-verb local `version`.
url="https://github.com/tractorbeamai/porter/releases/download/${VERSION}/${asset}"
sums_url="https://github.com/tractorbeamai/porter/releases/download/${VERSION}/checksums.txt"

mkdir -p "$INSTALL_DIR"
cd "$INSTALL_DIR"

curl -fsSL --retry 5 --retry-connrefused -o "$asset" "$url"
curl -fsSL --retry 5 --retry-connrefused -o checksums.txt "$sums_url"

# Verify against the release-published checksum file. Required —
# we never install an asset whose checksum we can't confirm.
# Two-space anchor matches the BSD format porter writes
# (`<sha>  <basename>`); `shasum -a 256` works on both Linux and
# macOS runners (`sha256sum` is Linux-only).
grep "  $asset\$" checksums.txt | shasum -a 256 -c -

tar -xzf "$asset"
chmod +x porter
echo "path=$INSTALL_DIR/porter" >> "$GITHUB_OUTPUT"
echo "$INSTALL_DIR" >> "$GITHUB_PATH"
"$INSTALL_DIR/porter" --version
