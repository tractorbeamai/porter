# Artifact kinds

Each entry in `[[artifacts]]` declares one publishable output. porter
expands the entries into a GitHub Actions job matrix at release time;
the reusable [release.yml](../.github/workflows/release.yml) consumes
that matrix and dispatches one job per row to the right step block.

This page documents each kind: the porter.toml fields, the runner the
matrix row lands on, the workflow steps that act on it, and what's
implemented today.

## Implementation status

| Kind             | porter.toml | matrix expansion | release.yml steps | cosign signing | tested end-to-end |
| ---------------- | ----------- | ---------------- | ----------------- | -------------- | ----------------- |
| `cli-binary`     | ✓           | ✓                | ✓                 | ✓ (blob)       | ✓ (porter ships itself this way) |
| `oci-image`      | ✓           | ✓                | ✓                 | ✓ (by digest)  | ✗                 |
| `helm-chart`     | ✓           | ✓                | ✓                 | ✓ (by digest)  | ✗                 |
| `npm-package`    | ✓           | ✓                | skeleton          | — (n/a)        | ✗                 |
| `python-wheel`   | ✓           | ✓                | skeleton          | — (n/a)        | ✗                 |

"Skeleton" means the step block exists in `release.yml` and shells out
to a standard tool (`npm publish`, `maturin`), but no consumer has
dogfooded the path and the steps may need iteration. If you hit an issue
with one of these, file it — the intent is full coverage; we just
haven't burned that path in yet.

## Signing

Signing is **opt-in**. Add a `[signing]` block to `porter.toml` and every
*signable* artifact — `cli-binary`, `oci-image`, `helm-chart` — is signed
with [cosign] and gets a [SLSA Build Provenance v1] attestation, both
keyless via the release job's OIDC token. With no block, nothing is
signed.

```toml
[signing]
# Empty is enough — everything below is the default.
backend = "sigstore"   # "none" is an explicit off-switch
fulcio_url = "https://fulcio.sigstore.dev"
rekor_url  = "https://rekor.sigstore.dev"
```

`porter matrix` reads this block and stamps each signable row with
`sign = true` plus the Fulcio/Rekor endpoints; the cosign steps in
`release.yml` are gated on `matrix.sign`. The two modalities:

- **By digest** (`oci-image`, `helm-chart`): `cosign sign` + `cosign
  attest --type slsaprovenance1` against `<registry>@<sha256:…>`. The
  signature and attestation live in the registry next to the artifact.
  This is what a [Sigstore policy-controller `ClusterImagePolicy`][policy]
  admits on.
- **As detached bundles** (`cli-binary`): `cosign sign-blob` and `cosign
  attest-blob` produce `<name>-<target>.sig.bundle` and `.att.bundle`,
  uploaded to the GitHub Release. Verify with `cosign verify-blob
  --bundle …` and `cosign verify-blob-attestation --bundle …`.

`npm-package` and `python-wheel` are not cosign-signed: npm carries its
own provenance (`npm publish --provenance`) and PyPI has its own
attestation story; porter stays out of their way.

**The provenance subject is set by cosign**, not porter. porter emits
just the predicate (`porter attest --emit predicate`) — the build
identity, source repo, and invocation metadata a policy verifies — and
cosign computes the subject digest from the artifact it actually signs.

**Registry auth.** Signing `oci-image`/`helm-chart` writes signatures to
the registry, so the job needs push credentials. The reusable workflow
logs in to `ghcr.io` automatically with the workflow token. For any
other registry (ECR, Docker Hub, …) the reusable workflow can't supply
credentials generically — your calling workflow must obtain them (e.g.
`aws-actions/configure-aws-credentials` + `aws ecr get-login-password`
for ECR). File an issue if you need a first-class hook for this.

[cosign]: https://docs.sigstore.dev/cosign/overview/
[SLSA Build Provenance v1]: https://slsa.dev/spec/v1.0/provenance
[policy]: ../policy/cluster-image-policy.example.yaml

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
4. When signing is enabled: `cosign sign-blob` and `cosign attest-blob
   --type slsaprovenance1` (predicate from `porter attest --emit
   predicate`), uploading `<name>-<target>.sig.bundle` and `.att.bundle`
   to the Release. See [Signing](#signing).

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
   `provenance: false` (porter issues its own SLSA attestation instead
   of the buildkit-default one).
3. When signing is enabled: `cosign sign` + `cosign attest --type
   slsaprovenance1` against `<registry>@<digest>` (the digest from the
   build-push step). See [Signing](#signing).

**Authentication:** the reusable workflow logs in to `ghcr.io`
automatically with the workflow token. For ECR, Docker Hub, or other
registries, your calling workflow must obtain push credentials before
invoking porter's `release.yml` — see [Signing](#signing) for details.

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
2. `helm push dist/<chart>-<version>.tgz <registry>` — the pushed ref
   and digest are parsed from helm's output.
3. When signing is enabled: `cosign sign` + `cosign attest --type
   slsaprovenance1` against the pushed `<repo>@<digest>` (a chart in an
   OCI registry is just another OCI artifact). See [Signing](#signing).

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
