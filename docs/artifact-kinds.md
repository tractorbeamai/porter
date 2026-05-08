# Artifact kinds

Each entry in `[[artifacts]]` declares one publishable output. porter
expands the entries into a GitHub Actions job matrix at release time;
the reusable [release.yml](../.github/workflows/release.yml) consumes
that matrix and dispatches one job per row to the right step block.

This page documents each kind: the porter.toml fields, the runner the
matrix row lands on, the workflow steps that act on it, and what's
implemented today.

## Implementation status

| Kind             | porter.toml | matrix expansion | release.yml steps | tested end-to-end |
| ---------------- | ----------- | ---------------- | ----------------- | ----------------- |
| `cli-binary`     | ✓           | ✓                | ✓ (Phase B)       | ✓ (porter ships itself this way) |
| `oci-image`      | ✓           | ✓                | skeleton          | ✗                 |
| `helm-chart`     | ✓           | ✓                | skeleton          | ✗                 |
| `npm-package`    | ✓           | ✓                | skeleton          | ✗                 |
| `python-wheel`   | ✓           | ✓                | skeleton          | ✗                 |

"Skeleton" means the step block exists in `release.yml` and shells out
to a standard tool (`docker/build-push-action`, `helm`, `npm publish`,
`maturin`), but no consumer has dogfooded the path and the steps may
need iteration. If you hit an issue with one of these, file it — the
intent is full coverage; we just haven't burned that path in yet.

## `cli-binary`

Cross-compile a Rust binary, archive it as a `.tar.gz`, write a
SHA-256 line into `dist/checksums.txt`, and upload to the GitHub
Release. The `setup-porter` action (and any analogue you write for
your tool) consumes the checksum file to verify downloads.

```toml
[[artifacts]]
kind = "cli-binary"
name = "porter"          # appears in the asset filename and matrix id
package = "porter-cli"   # cargo package name
targets = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
]
```

**Matrix expansion:** one row per target. Runner is picked
automatically — `ubuntu-latest` for x86_64 Linux, `ubuntu-24.04-arm`
for aarch64 Linux, `macos-13` for x86_64 macOS, `macos-14` for
aarch64 macOS. Any unknown target falls back to `ubuntu-latest` and
fails loudly if the runner can't compile for it.

**Workflow steps** (`release.yml` `build` job, `if: matrix.kind ==
'cli-binary'`):
1. `rustup target add ${{ matrix.target }}`
2. `porter build cli-binary --name <name> --target <target>` — runs
   `cargo build --release --target <target>` against the configured
   package, archives the resulting binary, computes the sha256, and
   appends to `dist/checksums.txt`.
3. `gh release upload <tag> dist/<name>-<target>.tar.gz` and the
   per-row checksum file.
4. (Phase D) emit + sign attestation alongside the artifact.

**Asset name:** `<name>-<target>.tar.gz`. The archive contains a
single binary at the archive root named `<name>` (or override via
`--binary` on the build subcommand).

## `oci-image`

Container image built with `docker/build-push-action` and pushed to a
registry. One image per artifact entry, multi-arch via `platforms`.

```toml
[[artifacts]]
kind = "oci-image"
name = "api"                                      # matrix id
context = "rust/"                                 # docker build context
dockerfile = "rust/bins/api/Dockerfile"           # path to the Dockerfile
registry = "ghcr.io/tractorbeamai/api"            # full registry path (no tag)
platforms = ["linux/amd64", "linux/arm64"]        # default: amd64+arm64
```

**Matrix expansion:** one row per artifact. Runner: `ubuntu-latest`.

**Workflow steps** (`if: matrix.kind == 'oci-image'`):
1. `docker/setup-buildx-action@v3`
2. `docker/build-push-action@v6` with `tags: <registry>:<vX.Y.Z>`,
   `provenance: false` (Phase D will issue our own attestations
   instead of the buildkit-default ones).

**Authentication:** the workflow assumes the runner has push access
via the workflow's `GITHUB_TOKEN` if `registry` is `ghcr.io/...`. For
DockerHub or other registries you'll need a `docker/login-action` step
in your wrapper workflow (the porter reusable doesn't add one yet —
this is part of "skeleton"). File an issue if you need a hook for
this; we'll add one.

## `helm-chart`

Package a chart with `helm package` and push it to an OCI registry.

```toml
[[artifacts]]
kind = "helm-chart"
name = "platform"                               # matrix id
chart = "deploy/helm/platform"                  # path to the chart directory
registry = "oci://ghcr.io/tractorbeamai/charts" # OCI registry (note `oci://` prefix)
```

**Matrix expansion:** one row per artifact. Runner: `ubuntu-latest`.

**Workflow steps** (`if: matrix.kind == 'helm-chart'`):
1. `helm package <chart> --version <vX.Y.Z without 'v'> --app-version
   <same> -d dist`
2. `helm push dist/<chart>-<version>.tgz <registry>`

**Note:** `helm package --version` rejects a leading `v`, so the
workflow strips it (`version="${TAG#v}"`). Your chart's `Chart.yaml`
should be listed under `[[versioned_files]]` with `type =
"helm-chart"` so it's bumped in lockstep with the tag — that's
porter's whole point.

## `npm-package`

Publish a JavaScript package to a registry via `npm publish`.

```toml
[[artifacts]]
kind = "npm-package"
name = "sdk"                                # matrix id
path = "ts/packages/sdk"                    # directory containing package.json
registry = "https://registry.npmjs.org"     # default; pass a custom URL otherwise
```

**Matrix expansion:** one row per artifact. Runner: `ubuntu-latest`.

**Workflow steps** (`if: matrix.kind == 'npm-package'`):
1. Write a `.npmrc` in `path` that auths against `registry` using the
   `NPM_TOKEN` secret.
2. `npm publish --access public` from `path`.

**Authentication:** the calling workflow must pass an `NPM_TOKEN`
secret (porter's release.yml reads it via
`secrets.NPM_TOKEN`). If `NPM_TOKEN` isn't set the step errors loudly
rather than silently producing an unauthenticated publish.

The `package.json` should be listed under `[[versioned_files]]` so
its version moves in lockstep with the tag.

## `python-wheel`

Build a Python wheel with `maturin` (PyO3-style native crates) and
upload it to the GitHub Release. PyPI publishing is intentionally
not in scope yet — wheels land on the release page where downstream
infrastructure can pick them up.

```toml
[[artifacts]]
kind = "python-wheel"
name = "client"           # matrix id
path = "py/client"        # directory containing pyproject.toml
```

**Matrix expansion:** one row per artifact. Runner: `ubuntu-latest`.

**Workflow steps** (`if: matrix.kind == 'python-wheel'`):
1. `pipx install maturin`
2. `maturin build --release --out ../dist` from `path`.

The wheel ends up in `dist/` and is uploaded with the rest of the
release assets.

## Mixing kinds in one repo

A polyglot repo will typically declare several entries:

```toml
[[artifacts]]
kind = "cli-binary"
name = "mytool"
package = "mytool-cli"
targets = ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"]

[[artifacts]]
kind = "oci-image"
name = "api"
context = "."
dockerfile = "Dockerfile"
registry = "ghcr.io/example/api"

[[artifacts]]
kind = "helm-chart"
name = "platform"
chart = "deploy/helm/platform"
registry = "oci://ghcr.io/example/charts"
```

The matrix expands to four rows (two `cli-binary` targets + one
`oci-image` + one `helm-chart`); each lands on its appropriate runner
and runs its kind-specific step block. They run in parallel because
the matrix is `fail-fast: false`.

## Extending with a new kind

Adding a new artifact kind is three changes:

1. New variant in `ArtifactConfig` ([crates/porter-core/src/config.rs](../crates/porter-core/src/config.rs)).
2. New match arm in `build_matrix` ([crates/porter-core/src/matrix.rs](../crates/porter-core/src/matrix.rs)).
3. New step block in [release.yml](../.github/workflows/release.yml)
   gated on `matrix.kind == '<new-kind>'`.

The matrix row carries arbitrary metadata via the kind-specific
`Option<...>` fields on `MatrixRow`; if your new kind needs a field
no other kind has, add it there. The fields are flattened to JSON so
the workflow accesses them as `matrix.<field>`.
