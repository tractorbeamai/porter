# Runbooks

Recovery procedures for the failure modes that actually happen with
porter in production. Each entry is a problem statement, what's
happening underneath, and the resolution.

## Rolling Version PR has merge conflicts

**Symptom.** The `Version Packages` PR shows "This branch has
conflicts that must be resolved" against `main`.

**Cause.** Something else landed on main that touched the same lines
the Version PR rewrites — usually `Cargo.lock` (concurrent dependency
updates) or the changelog header. The Version PR's branch is
rewritten in-place by `version.yml` on every push to main, so this
should self-heal on the next push — but a stale state can persist if
nothing's pushed for a while.

**Fix.** Push any commit to `main` (even an unrelated one) and the
next `version.yml` run regenerates the version branch from scratch.
If you need to force it without a real commit, re-run the latest
`version.yml` workflow from the Actions tab — the workflow is idempotent
and will rewrite the branch.

If the conflict is structural (e.g., a component's version source
referenced a file that's been deleted), fix the structural issue on
main first, then let the rolling PR refresh.

## A release fails partway through (tag pushed, build flaked)

**Symptom.** `porter-release.yml` (or your consumer `release.yml`)
fired on the `vX.Y.Z` tag, the tag is on the remote, but one or more
build matrix rows failed before the publish job ran. The GitHub
Release doesn't exist yet, or exists with missing assets.

**Fix.** Re-trigger the release via `workflow_dispatch`:

1. Actions tab → the release workflow → "Run workflow".
2. Set the `tag` input to the existing `vX.Y.Z`.
3. Run it.

The workflow is idempotent: matrix rows that already succeeded re-do
their work cheaply (Rust incremental builds), and the publish job
uses `gh release upload --clobber` so a partially-populated release
gets filled in rather than failing on conflict.

If a matrix row is failing for a deterministic reason (broken
target, missing dependency), fix the root cause on main and cut a
new release — don't try to re-run the broken row repeatedly.

## A bad release shipped

**Symptom.** `vX.Y.Z` is tagged, GitHub Release exists, binaries are
out, but the release is broken (corrupt binary, missing feature, etc.).

**Fix.** Two paths, depending on whether anyone's downloaded it yet.

### If nothing depends on it yet (no consumer pins to `vX.Y.Z`)

Delete and re-cut.

1. Delete the GitHub Release (UI or `gh release delete vX.Y.Z`).
2. Delete the remote tag. This must go through the App identity
   because of the tag-protection ruleset:

   ```sh
   # Mint an App installation token (locally, with a private key in
   # hand). Then:
   curl -fsSL -X DELETE \
     -H "Authorization: Bearer $INSTALLATION_TOKEN" \
     "https://api.github.com/repos/$ORG/$REPO/git/refs/tags/vX.Y.Z"
   ```

   You cannot delete the tag from a developer identity; the ruleset
   rejects deletes the same way it rejects pushes.

3. Fix main, push the tag again (via the App), let the release
   workflow re-run.

### If something already depends on it

Ship `vX.Y.Z+1` with the fix. Don't move `vX.Y.Z` — moving an
already-consumed tag breaks reproducibility for everyone pinning to
that exact ref. The floating `v0` tag will auto-advance to the new
release.

If the bad release was severe enough to require yanking (e.g., it
shipped a vulnerability), mark the GitHub Release as a pre-release
or add a banner in the release notes pointing at the replacement,
and notify consumers directly. The binaries themselves stay
downloadable — GitHub doesn't delete release assets when you delete
the Release object, only the listing.

## Tag push silently does nothing

**Symptom.** Someone runs `git tag vX.Y.Z && git push origin vX.Y.Z`,
the push appears to succeed locally, but no tag appears on the remote
and no release workflow fires.

**Cause.** The tag-protection ruleset rejected the push and `git
push` exited non-zero, but the developer didn't read the error.

**Check.** Re-run the push and read the output. Expected:

```
remote: error: GH013: Repository rule violations found for refs/tags/vX.Y.Z.
remote: - Cannot create ref due to rule violations
```

This is correct behavior — only the porter App can create `v*` tags.
Cut a release through the Version PR loop.

## `merge-when-ready` label doesn't auto-merge

**Symptom.** A PR has the `merge-when-ready` label, all checks are
green, and bulldozer doesn't merge.

**Common causes**, in order of likelihood:

- **Stale check name.** Your `.bulldozer.yml` requires e.g.
  `policy-bot: main`, but the PR's policy-bot check is named after a
  different base branch (because the PR was originally stacked, or
  was retargeted but policy-bot hasn't re-evaluated). Push any commit
  to the PR branch — even an empty `git commit --allow-empty -m
  "refresh policy-bot"` — to trigger re-evaluation.
- **Bulldozer hasn't seen a fresh event.** Bulldozer is webhook-driven
  and doesn't poll. If the label was applied before `.bulldozer.yml`
  existed in the repo, bulldozer saw the event with no config and
  moved on. Toggle the label off and on again to re-fire the event.
- **Merge conflict.** `gh pr view <n> --json mergeable` will report
  `CONFLICTING`. Rebase or, for Renovate PRs, tick the
  `<!-- rebase-check -->` checkbox in the PR body.
- **Required status not green.** Your `required_statuses` in
  `.bulldozer.yml` lists a check name that doesn't exist on the PR.
  Compare against `gh pr checks <n>` output.

## `setup-porter` checksum verification fails

**Symptom.** A CI job using `setup-porter` fails with `Bad checksum`
or `<asset>: FAILED`.

**Cause.** The downloaded binary's SHA-256 doesn't match the
`checksums.txt` published with the release. Either the asset is
corrupted (rare; investigate and report) or the runner downloaded a
partial file.

**Fix.** Re-run the failing job. If it persists across re-runs:

- Switch to SHA-pinned `setup-porter@<commit>` and `version: vX.Y.Z`
  to rule out floating-tag drift.
- Inspect the release: `gh release view vX.Y.Z --json assets`; compare
  the asset sizes to what your runner downloaded.
- If the checksums.txt itself is wrong, the release needs to be
  re-cut (see "A bad release shipped" above).
