# porter

Release-cutting tool for polyglot monorepos. One `vX.Y.Z` tag bumps every
version-bearing file in the repo atomically — Cargo workspaces, Helm
charts, `package.json`s, Terraform pins — then drives matrix builds and
GitHub Releases. Designed to be the sole privileged tagger of its host
repo, so releases originate from one identity and one only.

## Install

The CLI is distributed as a single static binary via GitHub Releases:

```sh
# in CI:
- uses: tractorbeamai/porter/actions/setup-porter@v0
  with:
    version: v0.1.0  # pin; `latest` is supported but discouraged
```

Locally, pull the matching tarball from the [releases page] and drop the
`porter` binary on your `PATH`.

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

[release]
tag_prefix = "v"
changelog = "CHANGELOG.md"
```

The full schema lives at [`schemas/porter.toml.json`](schemas/porter.toml.json);
your editor's TOML LSP will pick it up if pointed at that file.

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

## Repository layout

```
crates/porter-core/   # library (file format glue, version-sync, config)
crates/porter-cli/    # CLI entry point
actions/setup-porter/ # GitHub Action that downloads + checksum-verifies the binary
.github/workflows/    # ci.yml; reusable version.yml + release.yml
schemas/              # JSON Schema for porter.toml
```

## License

Apache 2.0. See [`LICENSE`](LICENSE).

[releases page]: https://github.com/tractorbeamai/porter/releases
