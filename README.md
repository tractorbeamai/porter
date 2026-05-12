# porter

Release-cutting tool for polyglot monorepos. One `vX.Y.Z` tag bumps every
version-bearing file in the repo atomically — Cargo workspaces, Helm
charts, `package.json`s, Terraform pins — then drives matrix builds and
GitHub Releases. Designed to be the sole privileged tagger of its host
repo, so releases originate from one identity and one only.

## Adopting porter in a new repo

End-to-end, in order. Each step is independently verifiable. For a
worked walkthrough with concrete commands and the click-through for
the GitHub App, see [`docs/getting-started.md`](docs/getting-started.md).

1. **Add a `porter.toml` at the repo root.** See [Configure](#configure)
   below. Start with just `[changesets]` and one `[[versioned_files]]`
   entry — you can grow the file later.

2. **Cut your first changeset.** Run `porter add --bump minor --summary
   "Initial release."` (or interactively). Commit it.

3. **Set up the porter App and the tag ruleset.** Follow
   [`app/README.md`](app/README.md): create the App in your org, install
   it on the repo, store the `PORTER_APP_ID` and `PORTER_APP_PRIVATE_KEY`
   repo secrets, then run `tools/install-ruleset.sh` to lock down
   `refs/tags/v*`. Until the ruleset is installed, anyone can `git tag &&
   git push` and bypass porter — the App is the entire trust boundary.

4. **Wire the rolling Version PR.** Add `.github/workflows/version.yml`
   in the consumer repo:

   ```yaml
   name: version
   on:
     push:
       branches: [main]
   permissions:
     contents: read
   jobs:
     mint:
       runs-on: ubuntu-latest
       outputs:
         token: ${{ steps.app-token.outputs.token }}
       steps:
         - id: app-token
           uses: actions/create-github-app-token@v2
           with:
             app-id: ${{ secrets.PORTER_APP_ID }}
             private-key: ${{ secrets.PORTER_APP_PRIVATE_KEY }}
     rolling-pr:
       needs: mint
       uses: tractorbeamai/porter/.github/workflows/version.yml@v0
       secrets:
         app-token: ${{ needs.mint.outputs.token }}
   ```

   The separate `mint` job exists because `actions/create-github-app-token`
   outputs the token from a step, and the reusable workflow consumes it
   as a workflow-level secret — forwarding via `needs.<job>.outputs.token`
   is the only way to bridge the two. The token stays masked in downstream
   logs.

5. **Wire the release workflow.** Add `.github/workflows/release.yml`:

   ```yaml
   name: release
   on:
     push:
       branches: [main]
       paths: [CHANGELOG.md]
   permissions:
     contents: read
   jobs:
     mint:
       runs-on: ubuntu-latest
       outputs:
         token: ${{ steps.app-token.outputs.token }}
       steps:
         - id: app-token
           uses: actions/create-github-app-token@v2
           with:
             app-id: ${{ secrets.PORTER_APP_ID }}
             private-key: ${{ secrets.PORTER_APP_PRIVATE_KEY }}
     release:
       needs: mint
       uses: tractorbeamai/porter/.github/workflows/release.yml@v0
       secrets:
         app-token: ${{ needs.mint.outputs.token }}
   ```

   The `paths: [CHANGELOG.md]` filter is the trigger — merging the
   Version PR changes the changelog, which fires this workflow. If you
   want the tag to push as soon as a version-bump commit lands on main
   (skipping the changelog-path heuristic), copy
   [`tag-on-version-merge.yml`](.github/workflows/tag-on-version-merge.yml)
   from this repo into your `.github/workflows/` — it asks `porter
   release tag` what tag the current state would carry and pushes if
   absent. Pick one or the other, not both.

6. **Merge the Version PR.** That tags `v0.0.1` (or whatever the bump
   computes), builds artifacts, and creates the GitHub Release.

The first release publishes a `CHANGELOG.md` if you don't have one.
After that the steady-state loop is just step 2 (author changesets) on
every release-worthy PR.

## Recommended setup (optional)

Everything below is opt-in. Skip any of it without losing the core
release loop. These are the same pieces porter dogfoods on itself.

### PR-status comment (changeset-bot equivalent)

Mirrors the comment @changesets-bot posts on each PR — pending
changesets, the bump, and the version this PR would produce on merge.
Non-blocking; absence of a changeset is a soft nudge, not an error.

```yaml
# .github/workflows/pr-status.yml
name: pr-status
on:
  pull_request:
    branches: [main]
permissions:
  contents: read
  pull-requests: write
jobs:
  status:
    uses: tractorbeamai/porter/.github/workflows/pr-status.yml@v0
```

### policy-bot + bulldozer (auto-merge on a label)

If your org runs [policy-bot](https://github.com/palantir/policy-bot)
and [bulldozer](https://github.com/palantir/bulldozer), copy porter's
[`.policy.yml`](.policy.yml) and [`.bulldozer.yml`](.bulldozer.yml) to
your repo root. They're a starting point, not a default — both encode
trust decisions (who can self-merge, what counts as approval) you
should review before adopting wholesale. Once installed, labeling a PR
`merge-when-ready` queues it for auto-merge as soon as policy-bot's
check is green.

### Renovate

porter ships [`renovate.json`](renovate.json) extending tractorbeam's
shared config and auto-labeling `patch` / `pin` / `digest` updates as
`merge-when-ready`. Adapt for your org's conventions; the
auto-labeling pattern is what makes Renovate PRs flow through the
bulldozer loop unattended.

## Install

The CLI is distributed as a single static binary via GitHub Releases:

```sh
# in CI:
- uses: tractorbeamai/porter/actions/setup-porter@v0
  with:
    version: v0.1.0  # pin; `latest` is supported but discouraged
```

Locally, pull the matching tarball from the [releases page] and drop
the `porter` binary on your `PATH`.

### Floating-tag semantics

Refs like `@v0` and `setup-porter@v0` are major-version floating tags
that porter's release workflow force-moves to the latest `v0.x.y` on
every release — same convention as `actions/checkout@v5`. You get
patch and minor updates automatically.

If you need a frozen ref, pin in this order of immutability:
- `setup-porter@<commit-sha>` for supply-chain-grade immutability of
  the install action itself, plus `version: vX.Y.Z` for a frozen CLI.
- `setup-porter@vX.Y.Z` plus `version: vX.Y.Z` for tag-level pinning
  (an attacker who compromised the repo could still move the tag, but
  it's immutable absent that).
- `setup-porter@v0` plus `version: latest` for follow-the-floating-major.

Reusable workflows (`version.yml`, `release.yml`, `pr-status.yml`)
follow the same convention — pin the `uses:` ref the same way.

## Configure

Create a `porter.toml` at the repo root:

```toml
[changesets]
directory = ".changeset"

[[versioned_files]]
type = "cargo-workspace"
path = "Cargo.toml"

[[versioned_files]]
type = "helm-chart"
path = "deploy/helm/platform/Chart.yaml"

[[versioned_files]]
type = "package-json"
path = "ts/packages/sdk/package.json"

[[versioned_files]]
type = "regex"
path = "deploy/main.tf"
pattern = 'platform_chart_revision\s*=\s*"(?P<version>v[0-9.]+)"'

# Artifacts the release workflow will build and publish. Optional —
# omit if you only need version-bumping. See docs/artifact-kinds.md
# for the full reference.
[[artifacts]]
kind = "cli-binary"
name = "mytool"
package = "mytool-cli"
targets = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
]

[release]
tag_prefix = "v"
changelog = "CHANGELOG.md"
```

The full schema lives at [`schemas/porter.toml.json`](schemas/porter.toml.json);
your editor's TOML LSP will pick it up if pointed at that file. For the
prose reference of each artifact kind, see
[`docs/artifact-kinds.md`](docs/artifact-kinds.md).

## Workflow

1. Author a changeset alongside any release-worthy PR:

   ```sh
   porter add --bump minor --summary "Add the attest subcommand."
   ```

   Or invoke `porter add` interactively. The result is a Markdown file in
   `.changeset/`.

2. On every push to `main`, the [version workflow](.github/workflows/version.yml)
   runs `porter version` and maintains a rolling "Version Packages" PR.
   The PR's diff is exactly the version bump and the corresponding
   `CHANGELOG.md` entry.

3. Merging the Version PR triggers the [release workflow](.github/workflows/release.yml),
   which tags `vX.Y.Z`, builds the artifacts declared in `porter.toml`,
   and creates a GitHub Release with the changelog body.

```sh
porter status                # what's pending and what version is next
porter version --dry-run     # show the diff without writing
porter version               # bump every versioned file, prepend changelog
porter release tag           # print the tag the next release would carry
porter release notes         # print the most recent changelog section body
```

## Subcommand reference

| Subcommand        | Purpose                                                            |
| ----------------- | ------------------------------------------------------------------ |
| `add`             | Author a `.changeset/*.md` file (interactive or via flags).        |
| `status`          | Print pending changesets, current version, and the computed next.  |
| `version`         | Apply pending changesets: bump every `versioned_files` entry, prepend `CHANGELOG.md`, and consume the changeset files. |
| `release tag`     | Print the tag of the next release (`<tag_prefix><current>`).       |
| `release notes`   | Print the body of the most recent changelog section.               |
| `matrix`          | Emit a GitHub Actions matrix derived from `[[artifacts]]`.         |
| `build cli-binary`| Cross-compile a CLI binary, archive it, and write a checksum line. |
| `attest`          | Emit an unsigned in-toto v1 Statement for an artifact (Phase D).   |

`status --json` and `matrix --compact` emit machine-readable shapes; see
[`docs/json-schemas.md`](docs/json-schemas.md) for the exact fields.

## How `next` is computed

porter uses the cargo / Changesets pre-1.0 convention — semver under
`1.0.0` treats a leading zero as the stability gate, so a "minor"
changeset against `0.5.2` produces `0.5.3` and a "major" changeset
produces `0.6.0`. Once you cross `1.0.0` the rules shift to ordinary
semver: minor → `1.x+1.0`, major → `(x+1).0.0`. Patch is always
`x.y.(z+1)`.

| Current | Bump  | Next  |
| ------- | ----- | ----- |
| 0.5.2   | patch | 0.5.3 |
| 0.5.2   | minor | 0.5.3 |
| 0.5.2   | major | 0.6.0 |
| 1.2.3   | patch | 1.2.4 |
| 1.2.3   | minor | 1.3.0 |
| 1.2.3   | major | 2.0.0 |

If a release-worthy PR shouldn't actually move the version (e.g. a
docs-only PR that needs to be marked as part of the next release),
author a `bump: patch` changeset; pre-1.0, it's the smallest move.

## Versioned-file kinds

- **`cargo-workspace`** — rewrites `[workspace.package].version` in a
  Cargo workspace manifest, preserving comments and field ordering.
- **`helm-chart`** — rewrites top-level `version` and (optionally)
  `appVersion` in a `Chart.yaml`. Targeted regex rewrite that leaves
  field order, quoting, and inline comments alone.
- **`package-json`** — rewrites the top-level `"version"` in a
  `package.json`, walking the JSON structurally so a nested `"version"`
  inside `dependencies` is not touched.
- **`regex`** — fallback for arbitrary files. Pattern must contain a
  named capture group `(?P<version>...)`; the matched substring is
  replaced. A leading `v` in the captured value is preserved.

## Troubleshooting

**`versioned files disagree on current version: <a> reports X, <b>
reports Y`** — drift between two of your `[[versioned_files]]` entries.
porter refuses to guess which one is correct. Bring them back into
agreement (usually by hand-editing the lagging one to match the others)
and rerun. Drift is exactly the bug porter exists to prevent, so this
error is intentional.

**`porter.toml has no [[versioned_files]] entries`** — you have a
`porter.toml` but no version-bearing files declared. Add at least one
`[[versioned_files]]` block; without one, porter has nothing to bump.

**Tag push rejected by ruleset** — the porter App ruleset is doing its
job. Check that the workflow that's pushing the tag has the correct
`secrets: app-token` (a porter App installation token, not
`GITHUB_TOKEN`). If you're trying to tag manually as a developer, you
can't — that's the point of the ruleset; cut a release through the
Version PR loop instead.

**`setup-porter` fails with `Bad checksum` or `<asset>: FAILED`** —
the binary's SHA-256 doesn't match the release-published
`checksums.txt`. Either the release is corrupted (rare; report it) or
your runner downloaded a partial asset (`curl --retry 5` is already in
the action). Re-run the failing job; if it persists, switch to a
SHA-pinned `version:` and a SHA-pinned `setup-porter@<commit>` to rule
out floating-tag drift.

**`could not find porter.toml`** — porter walks up from the current
working directory looking for `porter.toml`. Pass `--config <path>` if
your CI step runs from a subdirectory, or `cd` to the repo root first.

**Pre-1.0 minor changesets produce only a patch bump** — that's
intentional. See [How `next` is computed](#how-next-is-computed) above.

## Repository layout

```
crates/porter-core/   # library (file format glue, version-sync, config)
crates/porter-cli/    # CLI entry point
actions/setup-porter/ # GitHub Action that downloads + checksum-verifies the binary
.github/workflows/    # ci.yml; reusable version.yml + release.yml
schemas/              # JSON Schema for porter.toml
docs/                 # consumer-facing reference (artifact kinds, JSON schemas, phases)
app/                  # GitHub App manifest + setup instructions
```

## More

- [`docs/artifact-kinds.md`](docs/artifact-kinds.md) — every `[[artifacts]]` kind, what it expects, and what's implemented today.
- [`docs/json-schemas.md`](docs/json-schemas.md) — the exact shapes of `porter status --json` and `porter matrix --compact`.
- [`docs/phases.md`](docs/phases.md) — the A/B/C/D/E phase plan referenced in commit messages and code comments.
- [`docs/getting-started.md`](docs/getting-started.md) — end-to-end fresh-adoption walkthrough.
- [`docs/runbooks.md`](docs/runbooks.md) — recovery procedures for the failure modes that actually happen (merge conflicts, mid-release failures, rollback).
- [`app/README.md`](app/README.md) — GitHub App + ruleset setup.
- [`.changeset/README.md`](.changeset/README.md) — changeset authoring rules.
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — running tests, dogfooding porter on porter.
- [`SECURITY.md`](SECURITY.md) — vulnerability disclosure.

## License

Apache 2.0. See [`LICENSE`](LICENSE).

[releases page]: https://github.com/tractorbeamai/porter/releases
