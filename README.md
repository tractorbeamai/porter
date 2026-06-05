# porter

[![ci](https://github.com/tractorbeamai/porter/actions/workflows/ci.yml/badge.svg)](https://github.com/tractorbeamai/porter/actions/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/tractorbeamai/porter)](https://github.com/tractorbeamai/porter/releases)
[![license](https://img.shields.io/github/license/tractorbeamai/porter)](LICENSE)

Release-cutting tool for polyglot monorepos. One `vX.Y.Z` tag bumps
every version-bearing file in lockstep, then drives matrix builds and
GitHub Releases — from one demonstrable identity, with no humans in
the loop.

## Highlights

- **[Changesets](#changesets)** make every release-worthy change
  explicit, reviewable, and aggregable.
- **[Atomic version bumps](#version-bumps)** across Cargo workspaces,
  Helm charts, `package.json`s, Terraform pins — drift between files
  is detected and refused, not silently papered over.
- **[Rolling Version PR](#releases)** in the @changesets style: a
  single PR shows what the next release would be, updated on every
  push to main. Merging it cuts the release.
- **[Cross-compiled artifacts](#artifacts)** published to GitHub
  Releases with deterministic checksums.
- **[Sole privileged tagger](#privileged-tagger)** — a single-purpose
  GitHub App locked behind a repo ruleset is the only identity that
  can push `v*` tags. Every release in history is demonstrably from
  porter.
- **[Sticky PR-status comments](#pr-status-comments)** show pending
  changesets and the version each PR would produce on merge.
- **Self-hosted by design.** porter releases itself through its own
  loop — no parallel toolchain to maintain.
- **Single static binary.** No daemon, no service, nothing to run on
  consumer infrastructure beyond the GitHub App for identity.

## Installation

In CI:

```yaml
- uses: tractorbeamai/porter/actions/setup-porter@v0
  with:
    version: v0.1.0  # pin; `latest` is supported but discouraged
```

Locally, grab a tarball from the [releases page] and drop `porter` on
your `PATH`.

See [floating-tag semantics](#floating-tag-semantics) for the pinning
options if you need supply-chain immutability.

## Documentation

- **[Getting started](docs/getting-started.md)** — end-to-end
  fresh-adoption walkthrough (~20 minutes for the first repo).
- **[Configuration](#configuration)** — `porter.toml` reference.
- **[Runbooks](docs/runbooks.md)** — recovery procedures for the
  failure modes that actually happen.
- **[Artifact kinds](docs/artifact-kinds.md)** — every `artifact`
  kind, what it expects, and what's implemented today.
- **[Signing & trust model](docs/signing-and-trust.md)** — what a
  signature proves, who holds the signing identity, and owning your build.
- **[JSON schemas](docs/json-schemas.md)** — exact shapes of
  `porter status --json` and `porter matrix --compact`.
- **[Phases](docs/phases.md)** — the A/B/C/D/E plan referenced in
  commit messages.
- `porter --help` for the full CLI reference.

## Features

### Changesets

Author a changeset alongside any release-worthy PR. The summary lands
verbatim in the changelog, so write it like a release note.

```console
$ porter add --bump minor --summary "Add the attest subcommand."
wrote .changeset/add-the-attest-subcommand.md
```

The bump category is *user-visible impact*, not diff size. A one-line
fix that breaks compatibility is still `major`. A docs-only PR that
needs to ride the next release is still a `patch`.

See [`.changeset/README.md`](.changeset/README.md) for the file
format.

### Version bumps

Inspect what the next release would be:

```console
$ porter status
default: 0.5.2 -> 0.5.3 (minor)
  minor  .changeset/add-the-attest-subcommand.md  Add the attest subcommand.
```

Apply it. Each bumped group rewrites its version sources in lockstep, the
changesets are consumed, and the group's changelog gets the new section
prepended:

```console
$ porter version
bumped default: 0.5.2 -> 0.5.3 (minor)
  Cargo.toml
  deploy/chart/Chart.yaml
  ts/packages/sdk/package.json
  tags: mytool/v0.5.3
consumed 1 changeset file(s)
```

Drift detection is exactly the bug porter exists to prevent — if two version
sources *within a group* disagree on the current version, `porter version`
refuses to proceed and tells you which files disagree. (Different groups
holding different versions is expected — that's the point of groups.) Bring a
group's files back into agreement and rerun.

`status --json` and `matrix --compact` emit machine-readable shapes;
see [`docs/json-schemas.md`](docs/json-schemas.md).

### Versioned files

Built-in adapters:

- **`cargo-workspace`** — rewrites `[workspace.package].version`,
  preserving comments and field ordering.
- **`helm-chart`** — rewrites top-level `version` and (optionally)
  `appVersion` in `Chart.yaml`. Targeted regex rewrite that leaves
  field order, quoting, and inline comments alone.
- **`package-json`** — rewrites the top-level `"version"`, walking
  the JSON structurally so a nested `"version"` inside `dependencies`
  is not touched.
- **`regex`** — fallback for arbitrary files. Pattern must contain a
  named capture group `(?P<version>...)`; the matched substring is
  replaced. A leading `v` in the captured value is preserved.

A version source attaches to a component via its `type`/`path`; every
version-bearing component in a group moves together or not at all.

### Releases

A reusable workflow opens a rolling **Version Packages** PR on every
push to main, showing exactly the bump and changelog entry the next
release would carry. Merging that PR triggers the release workflow,
which tags `vX.Y.Z` via the porter App identity, fans out the build
matrix, and creates the GitHub Release.

```console
$ porter release tag
v0.5.3

$ porter release notes
### Features

- Add the attest subcommand.
```

Consumer wiring is two `.github/workflows/*.yml` files that mint a
porter App installation token and call porter's reusable workflows —
see [getting started](docs/getting-started.md) for the exact YAML.

### Artifacts

A component's `artifact` says what it builds and publishes:

```toml
[[group]]
name = "default"
components = [
  { id = "mytool", type = "cargo-workspace", path = "Cargo.toml", tag_prefix = "v",
    artifact = { kind = "cli-binary", package = "mytool-cli", targets = [
      "x86_64-unknown-linux-gnu",
      "aarch64-unknown-linux-gnu",
      "x86_64-apple-darwin",
      "aarch64-apple-darwin",
    ] } },
]
```

`porter matrix --kind cli-binary --compact` emits a GitHub Actions
matrix; `porter build cli-binary --target X` cross-compiles,
archives, and writes a deterministic SHA-256 line into
`dist/checksums.txt`. The release workflow consumes both.

cli-binary is end-to-end today. `oci-image`, `helm-chart`,
`npm-package`, and `python-wheel` kinds are scaffolded — see
[`docs/artifact-kinds.md`](docs/artifact-kinds.md) for the status
table.

### Signing

Opt-in. Add a `[signing]` block — empty is enough — and every signable
artifact (container images, Helm charts, CLI binaries) is signed with
[cosign] and gets a SLSA Build Provenance v1 attestation, keyless via
the release job's OIDC token (no keys to manage). Images and charts are
signed by registry digest; binaries get detached
`.sig.bundle`/`.att.bundle` files on the Release. npm and Python
artifacts are left to their ecosystems' own provenance.

```toml
[signing]
# Empty is enough: keyless Sigstore against public Fulcio/Rekor.
# backend = "sigstore"   # the default; "none" is an explicit off-switch
```

With no `[signing]` block, releases aren't signed — zero config until
you want it. Signing images/charts pushes signatures to the registry,
so the job needs push credentials: the reusable workflow logs in to
`ghcr.io` automatically, while other registries (ECR, Docker Hub) need a
login step in your calling workflow. See
[`docs/artifact-kinds.md`](docs/artifact-kinds.md#signing) for
verification commands and the admission-policy example in
[`policy/`](policy/cluster-image-policy.example.yaml). The signing identity
is the workflow that runs cosign — the reusable workflow signs as porter, or
compose `porter-sign` to sign under your own repo's identity;
[`docs/signing-and-trust.md`](docs/signing-and-trust.md) covers the trust
model and both paths.

[cosign]: https://docs.sigstore.dev/cosign/overview/

### Privileged tagger

A single-purpose [GitHub App](app/README.md) holds the only identity
allowed to push release tags. A [repo ruleset](tools/install-ruleset.sh)
enforces it: any push to `refs/tags/v*` from a non-App identity is
rejected. Pattern borrowed from
[Palantir's Autorelease](https://blog.palantir.com/how-palantir-secures-source-control-105c49079eae).

The boundary is verifiable end-to-end: every release tag in the
repo's history demonstrably originates from one App installation,
because no other identity could have created it.

Phase D extends this with [Sigstore attestations](docs/phases.md#phase-d--attestation)
chained back to the App identity.

#### Credential policy

porter's workflows authenticate with one of two credentials, chosen by a
single rule:

- **Writing a protected git ref** — pushing/moving a branch or any `v*` tag —
  uses the **porter App token** (`porter[bot]`). It is the only identity the
  rulesets allow to write those refs, and because it is a real identity its
  pushes *trigger* downstream workflows (a release tag must fire the release).
  Always mint the token in the job that uses it: `create-github-app-token`
  revokes the token when its job ends, so a token minted in one job and handed
  to another (or to a called workflow) arrives already-invalid. porter's
  in-repo workflows mint via the [`mint-porter-token`](actions/mint-porter-token)
  composite; the reusables that run in a consumer's checkout call
  `create-github-app-token` directly. Where the token-using job also runs build
  or third-party steps (`porter-release`'s floating-tag move), minting is split
  into its own job so the App key isn't co-located with them.
- **Everything else** — `gh release` create/upload on an existing tag, PR-status
  comments, registry login, artifact upload, and OIDC signing (`id-token:
  write`, cosign) — uses **`GITHUB_TOKEN`**. It's auto-scoped, expires per job,
  and never needs ruleset bypass.
- **Suppressing an unwanted trigger** is done with the workflow's *trigger
  pattern*, not the token choice. For example `porter-release.yml` listens on
  `tags: ["v*.*.*"]` (full semver only) so that force-moving the floating `v0`
  major tag — itself an App-token write — doesn't re-trigger the release.

Each credential-bearing step is tagged in the workflow with a
`# credential: app-token — <why>` or `# credential: github.token — <why>`
comment pointing back to this rule. The one deliberate exception
(`release.yml` authoring the GitHub Release as `porter[bot]` for provenance)
says so inline.

### PR-status comments

A reusable workflow posts a sticky comment on every PR showing
pending changesets and the version the PR would produce on merge:

```yaml
# .github/workflows/pr-status.yml
jobs:
  status:
    uses: tractorbeamai/porter/.github/workflows/pr-status.yml@v0
```

Non-blocking by design — a missing changeset on a docs / CI /
refactor PR is fine. The comment is informational.

## Configuration

A `porter.toml` declares one or more **groups**. A group is a release line:
its components share one version, move together, and cut their own tags. A
**component** bundles a *version source* (the file whose version string is
rewritten) and an optional *artifact* (what's built and published); it may be
either or both. Components in different groups version independently.

```toml
[changesets]
directory = ".changeset"

# The application: Rust workspace + its container image, one version line.
[[group]]
name = "app"
components = [
  { id = "app", type = "cargo-workspace", path = "Cargo.toml",
    artifact = { kind = "cli-binary", package = "app-cli" } },
  { id = "api", artifact = { kind = "oci-image", context = ".",
    dockerfile = "Dockerfile", registry = "ghcr" } },
]

# The SDK ships for two languages in lockstep on its own line.
[[group]]
name = "sdk"
changelog = "sdk/CHANGELOG.md"
components = [
  { id = "py-sdk", type = "regex", path = "py/pyproject.toml",
    pattern = '(?m)^version = "(?P<version>[^"]+)"',
    artifact = { kind = "python-wheel", path = "py" } },
  { id = "ts-sdk", type = "package-json", path = "ts/packages/sdk/package.json",
    artifact = { kind = "npm-package", path = "ts/packages/sdk" } },
]

# Named registries an artifact's `registry` field references by name. A bare
# URL also works (anonymous, no auth).
[registries.ghcr]
kind = "oci"
url  = "ghcr.io/acme"
auth = { type = "github-token" }

[release]
changelog = "CHANGELOG.md"   # default for groups without their own
version_pr_title = "Version Packages: {version}"   # {version} filled when one group bumps
```

A changeset names the group(s) it bumps (`porter add --group sdk`); each group
computes its own next version, writes its own changelog, and cuts one tag per
published component (`py-sdk/v0.4.1`, `ts-sdk/v0.4.1`, …). Set a component's
`tag_prefix` to override the default `<id>/v` stem (e.g. `"v"` for bare
`vX.Y.Z` in a single-group repo).

The full schema lives at [`schemas/porter.toml.json`](schemas/porter.toml.json);
your editor's TOML LSP will pick it up if pointed at that file.

## How `next` is computed

porter uses the cargo / Changesets pre-1.0 convention. Under `1.0.0`,
[SemVer's initial-development clause][semver-0] treats the public API as
unstable, so digit significance shifts left: a "minor" changeset against
`0.5.2` produces `0.5.3` (patch-position bump) and a "major" changeset
produces `0.6.0` (minor-position bump). Once you cross `1.0.0` the rules
shift to ordinary semver.

[semver-0]: https://semver.org/#spec-item-4

| Current | Bump  | Next  |
| ------- | ----- | ----- |
| 0.5.2   | patch | 0.5.3 |
| 0.5.2   | minor | 0.5.3 |
| 0.5.2   | major | 0.6.0 |
| 1.2.3   | patch | 1.2.4 |
| 1.2.3   | minor | 1.3.0 |
| 1.2.3   | major | 2.0.0 |

If a release-worthy PR shouldn't actually move the version (docs-only
PRs that need to be marked as part of the next release), author a
`bump: patch` changeset; pre-1.0, it's the smallest move.

## Subcommand reference

| Subcommand        | Purpose                                                            |
| ----------------- | ------------------------------------------------------------------ |
| `add`             | Author a `.changeset/*.md` file (interactive or via flags).        |
| `status`          | Print pending changesets, current version, and the computed next.  |
| `version`         | Apply pending changesets: bump each group's version sources, prepend its changelog, and consume the changeset files. |
| `release tag`     | Print every published component's tag, one per line.               |
| `release notes`   | Print the body of the most recent changelog section (`--group` for a group's changelog). |
| `matrix`          | Emit a GitHub Actions matrix of every group's artifacts.           |
| `build cli-binary`| Cross-compile a CLI binary, archive it, and write a checksum line. |
| `attest`          | Emit unsigned SLSA provenance for an artifact — a complete in-toto v1 Statement (`--emit statement`, default) or just the predicate for `cosign attest` to sign (`--emit predicate`). |

Run `porter <subcommand> --help` for flags.

## Floating-tag semantics

Refs like `@v0` and `setup-porter@v0` are major-version floating
tags that porter's release workflow force-moves to the latest
`v0.x.y` on every release — same convention as
`actions/checkout@v5`. You get patch and minor updates automatically.

If you need a frozen ref, pin in this order of immutability:

- `setup-porter@<commit-sha>` plus `version: vX.Y.Z` —
  supply-chain-grade immutability.
- `setup-porter@vX.Y.Z` plus `version: vX.Y.Z` — tag-level pinning
  (an attacker who compromised the repo could still move the tag,
  but it's immutable absent that).
- `setup-porter@v0` plus `version: latest` — follow the floating
  major.

Reusable workflows (`version.yml`, `release.yml`, `pr-status.yml`)
follow the same convention; pin the `uses:` ref the same way.

## Repository layout

```
crates/porter-core/   # library (file format glue, version-sync, config)
crates/porter-cli/    # CLI entry point
actions/setup-porter/ # GitHub Action that downloads + checksum-verifies the binary
.github/workflows/    # ci.yml; reusable version.yml + release.yml + pr-status.yml
schemas/              # JSON Schema for porter.toml
docs/                 # consumer-facing reference (getting-started, runbooks, artifact kinds, ...)
app/                  # GitHub App spec + setup instructions
tools/                # install-ruleset.sh and other shell helpers
```

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for local development, the
test layout, and the dogfooding-porter-on-porter loop. Security
reports go through [`SECURITY.md`](SECURITY.md).

## License

Apache 2.0. See [`LICENSE`](LICENSE).

[releases page]: https://github.com/tractorbeamai/porter/releases
