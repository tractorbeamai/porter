#!/usr/bin/env bash
# The body of the `porter-tag` composite action, factored out of action.yml so
# it can be unit-tested with bats and statically checked with shellcheck.
#
# Pushes every pending component tag as porter[bot], emits the build matrix and
# the unique tag list as step outputs, and opens one GitHub Release per
# (tag, group) with that group's changelog as notes. Idempotent: tags and
# Releases that already exist are skipped, so a re-run is safe.
#
# Inputs (env vars):
#   GH_TOKEN            — porter App installation token (push + Release author).
#   GITHUB_REPOSITORY   — `owner/repo` (GH-provided); the push target.
#   GITHUB_OUTPUT       — file the runner reads for outputs.
#
# Requires `porter`, `git`, `gh`, and `jq` on PATH, and a `fetch-depth: 0`
# checkout so existing tags resolve.

set -euo pipefail

git config user.name "porter[bot]"
git config user.email "porter[bot]@users.noreply.github.com"

# Cut every tag porter wants that the remote doesn't already have.
refs=()
while IFS= read -r tag; do
  [[ -z "$tag" ]] && continue
  if git rev-parse "refs/tags/$tag" >/dev/null 2>&1; then
    echo "tag $tag already exists; skipping"
  else
    git tag -a "$tag" -m "Release $tag"
    refs+=("refs/tags/$tag")
  fi
done < <(porter release tag)

# Push only the new tags. Guarded by length so the (possibly empty) array is
# never expanded under `set -u`. Explicit token URL scopes the credential to
# this push (checkout set persist-credentials: false).
if [[ ${#refs[@]} -gt 0 ]]; then
  git push "https://x-access-token:${GH_TOKEN}@github.com/${GITHUB_REPOSITORY}.git" "${refs[@]}"
fi

# Emit the build matrix + a has-artifacts flag for downstream jobs.
matrix=$(porter matrix --compact)
echo "matrix=$matrix" >> "$GITHUB_OUTPUT"
rows=$(echo "$matrix" | jq '.include | length')
if [[ "$rows" -gt 0 ]]; then
  echo "has-artifacts=true" >> "$GITHUB_OUTPUT"
else
  echo "has-artifacts=false" >> "$GITHUB_OUTPUT"
fi

# The unique tags this release cuts, for a finalize/manifest job.
tags=$(echo "$matrix" | jq -r '.include[].tag' | sort -u)
{
  echo "tags<<__PORTER_EOF__"
  echo "$tags"
  echo "__PORTER_EOF__"
} >> "$GITHUB_OUTPUT"

# One Release per unique (tag, group), notes from that group's changelog.
echo "$matrix" | jq -rc '.include[] | {tag, group}' | sort -u | while read -r row; do
  tag=$(jq -r '.tag' <<< "$row")
  group=$(jq -r '.group' <<< "$row")
  if gh release view "$tag" >/dev/null 2>&1; then
    echo "release $tag already exists"
  else
    notes_file=$(mktemp)
    porter release notes --group "$group" > "$notes_file" || true
    gh release create "$tag" --title "$tag" --notes-file "$notes_file"
  fi
done
