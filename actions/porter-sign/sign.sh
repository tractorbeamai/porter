#!/usr/bin/env bash
# The body of the `porter-sign` composite action, factored out of action.yml
# so it can be unit-tested with bats and statically checked with shellcheck.
# action.yml invokes this once with everything passed via env vars.
#
# Flow: (1) when SIGN=true and the kind is signable, cosign-sign the artifact
# and attach porter's SLSA provenance via cosign attest; (2) always emit a
# `porter release record` and upload it to the component's Release.
#
# Inputs (env vars):
#   KIND          — oci-image | helm-chart | cli-binary | npm-package | python-wheel.
#   SIGN          — `true` to cosign-sign + attest; anything else records only.
#   NAME GROUP TAG VERSION ID — record identity; TAG also pins the predicate
#                   source-ref and is the Release uploads target.
#   REF           — `<repo>@<digest>` signing ref (oci-image, helm-chart).
#   REGISTRY      — registry/repository for the record (oci/helm/npm).
#   ARTIFACT      — blob path to sign (cli-binary); `.sig.bundle`/`.att.bundle`
#                   are written alongside it.
#   TARGET SHA256 — cli-binary record fields.
#   FULCIO_URL REKOR_URL — cosign endpoints; empty ⇒ cosign defaults (public).
#   GH_TOKEN      — token for `gh release upload`.

set -euo pipefail

: "${KIND:?porter-sign: KIND is required}"
: "${NAME:?porter-sign: NAME is required}"
: "${GROUP:?porter-sign: GROUP is required}"
: "${TAG:?porter-sign: TAG is required}"
: "${VERSION:?porter-sign: VERSION is required}"
: "${ID:?porter-sign: ID is required}"

# cosign keyless endpoint flags — omitted when unset so cosign falls back to
# the public Sigstore instances. Expanded at each call site with the `[@]+`
# guard so an empty array doesn't trip `set -u` on bash 3.2 (macOS, where the
# bats suite runs).
cosign_flags=()
[[ -n "${FULCIO_URL:-}" ]] && cosign_flags+=(--fulcio-url "$FULCIO_URL")
[[ -n "${REKOR_URL:-}" ]] && cosign_flags+=(--rekor-url "$REKOR_URL")

# ----- sign + attest (opt-in) -----------------------------------------
if [[ "${SIGN:-}" == "true" ]]; then
  case "$KIND" in
    oci-image | helm-chart)
      if [[ -z "${REF:-}" ]]; then
        echo "::error::porter-sign: ref (<repo>@<digest>) is required to sign a $KIND" >&2
        exit 1
      fi
      cosign sign --yes "${cosign_flags[@]+"${cosign_flags[@]}"}" "$REF"
      porter attest --emit predicate --source-ref "refs/tags/${TAG}" > predicate.json
      cosign attest --yes "${cosign_flags[@]+"${cosign_flags[@]}"}" \
        --predicate predicate.json --type slsaprovenance1 "$REF"
      ;;
    cli-binary)
      if [[ -z "${ARTIFACT:-}" ]]; then
        echo "::error::porter-sign: artifact path is required to sign a cli-binary" >&2
        exit 1
      fi
      sig="${ARTIFACT%.tar.gz}.sig.bundle"
      att="${ARTIFACT%.tar.gz}.att.bundle"
      cosign sign-blob --yes "${cosign_flags[@]+"${cosign_flags[@]}"}" --bundle "$sig" "$ARTIFACT"
      porter attest --emit predicate --source-ref "refs/tags/${TAG}" > predicate.json
      cosign attest-blob --yes "${cosign_flags[@]+"${cosign_flags[@]}"}" \
        --predicate predicate.json --type slsaprovenance1 --bundle "$att" "$ARTIFACT"
      gh release upload "$TAG" --clobber "$sig" "$att"
      ;;
    *)
      echo "::error::porter-sign: kind '$KIND' is not signable (use sign: 'false')" >&2
      exit 1
      ;;
  esac
fi

# ----- publish record (always) ----------------------------------------
# Identity + digest of what shipped, so the manifest is machine-readable rather
# than scraped from a tool's stdout. Routed by kind, mirroring the columns each
# kind carries.
args=(--kind "$KIND" --name "$NAME" --group "$GROUP" --tag "$TAG" --version "$VERSION")
case "$KIND" in
  oci-image)
    # REF is `<repo>@<digest>`; the record wants the registry + bare digest.
    args+=(--registry "$REGISTRY" --digest "${REF##*@}") ;;
  helm-chart)
    args+=(--registry "$REGISTRY" --digest "${REF##*@}") ;;
  cli-binary)
    args+=(--target "$TARGET" --sha256 "$SHA256" --asset "${NAME}-${TARGET}.tar.gz") ;;
  npm-package)
    args+=(--registry "$REGISTRY") ;;
esac

mkdir -p dist
porter release record "${args[@]}" > "dist/published-${ID}.json"
gh release upload "$TAG" --clobber "dist/published-${ID}.json"
