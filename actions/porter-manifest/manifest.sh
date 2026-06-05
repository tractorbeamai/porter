#!/usr/bin/env bash
# The body of the `porter-manifest` composite action, factored out of action.yml
# so it can be unit-tested with bats and statically checked with shellcheck.
#
# For each tag the release cut, this merges the per-row assets the build jobs
# uploaded into the canonical Release artifacts:
#   * checksums-<id>.txt  → checksums.txt   (verified by setup-porter)
#   * published-<id>.json → published.json  (porter's manifest of what shipped)
# and appends a per-release summary table to $GITHUB_STEP_SUMMARY.
#
# Trust model: only the porter App can upload to a Release (repo ruleset), so
# every per-row asset merged here is App-authored.
#
# Inputs (env vars):
#   TAGS          — newline-separated tags to finalize (porter-tag's `tags`).
#   GH_TOKEN      — token for the asset download/upload.
#   GITHUB_STEP_SUMMARY — file the runner renders as the job summary.
#
# Requires `porter`, `gh`, and `jq` on PATH.

set -euo pipefail

# Download every Release asset matching $1 (an ERE) for tag $2 into a fresh
# temp dir, echoed on stdout. Empty output ⇒ nothing matched.
download_matching() {
  local pattern="$1" tag="$2" workdir names asset
  names=$(gh release view "$tag" --json assets --jq '.assets[].name' | grep -E "$pattern" || true)
  [[ -z "$names" ]] && return 1
  workdir=$(mktemp -d)
  while IFS= read -r asset; do
    [[ -z "$asset" ]] && continue
    gh release download "$tag" --pattern "$asset" --dir "$workdir" --clobber
  done <<< "$names"
  echo "$workdir"
}

while IFS= read -r tag; do
  [[ -z "$tag" ]] && continue

  # Canonical checksums.txt — concatenate the per-row checksum files.
  if workdir=$(download_matching '^checksums-' "$tag"); then
    cat "$workdir"/checksums-*.txt | sort -u > "$workdir/checksums.txt"
    gh release upload "$tag" --clobber "$workdir/checksums.txt"
  fi

  # Canonical published.json — merge the per-row records (porter owns the schema
  # + ordering) and summarize what shipped.
  if workdir=$(download_matching '^published-.*\.json$' "$tag"); then
    porter release manifest "$workdir"/published-*.json > "$workdir/published.json"
    gh release upload "$tag" --clobber "$workdir/published.json"
    {
      echo "### $tag"
      echo
      echo "| kind | name | version | digest / sha256 |"
      echo "| --- | --- | --- | --- |"
      jq -r '.[] | "| \(.kind) | \(.name) | \(.version) | \(.digest // .sha256 // "—") |"' "$workdir/published.json"
      echo
    } >> "$GITHUB_STEP_SUMMARY"
  fi
done <<< "$TAGS"
