# Artifact kinds

A component's `artifact` declares one publishable output. porter expands
every group's artifact-bearing components into a GitHub Actions job matrix at
release time; the reusable [release.yml](../.github/workflows/release.yml)
consumes that matrix and dispatches one job per row to the right step block.

This page documents each kind: the `artifact` fields, the runner the matrix
row lands on, the workflow steps that act on it, and what's implemented today.
The component `id` supplies the artifact's name (there's no `name` field), and
the published tag is the component's tag (`<id>/v<version>`).

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
the registry, so the job needs push credentials — declare them on the
`[registries]` entry's `auth` (see [Registries](#registries)), including
`aws-ecr` for AWS ECR. The reusable workflow logs in for you.

[cosign]: https://docs.sigstore.dev/cosign/overview/
[SLSA Build Provenance v1]: https://slsa.dev/spec/v1.0/provenance
[policy]: ../policy/cluster-image-policy.example.yaml

## `cli-binary`

Cross-compile a Rust binary, archive it as a `.tar.gz`, write a
SHA-256 line into `dist/checksums.txt`, and upload to the GitHub
Release. The `setup-porter` action (and any analogue you write for
your tool) consumes the checksum file to verify downloads.

```toml
[[group]]
name = "default"
components = [
  { id = "porter", type = "cargo-workspace", path = "Cargo.toml", tag_prefix = "v",
    artifact = { kind = "cli-binary", package = "porter-cli", targets = [
      "x86_64-unknown-linux-gnu",
      "aarch64-unknown-linux-gnu",
      "x86_64-apple-darwin",
      "aarch64-apple-darwin",
    ] } },
]
```

The component `id` (`porter`) appears in the asset filename and matrix id;
`package` is the cargo package name.

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
[registries.ghcr]
kind = "oci"
url  = "ghcr.io/tractorbeamai"
auth = { type = "github-token" }

[[group]]
name = "default"
components = [
  { id = "api", type = "cargo-workspace", path = "Cargo.toml",
    artifact = { kind = "oci-image",
      context = "rust/",                      # docker build context
      dockerfile = "rust/bins/api/Dockerfile",
      registry = "ghcr",                      # a [registries] name, or a bare repo URL
      platforms = ["linux/amd64", "linux/arm64"] } },
]
```

A named `oci` registry holds the host/org prefix; the image repo is
`<url>/<id>` (here `ghcr.io/tractorbeamai/api`). A bare URL is used as the full
repo as-is. The image tag is the bare `version` (e.g. `0.5.3`).

**Matrix expansion:** one row per component. Runner: `ubuntu-latest`.

**Workflow steps** (`if: matrix.kind == 'oci-image'`):
1. Registry login (see [Registries](#registries)).
2. `docker/setup-buildx-action@v3`
3. Resolve publish state: `docker buildx imagetools inspect <repo>:<version>`.
   If the version is already in the registry the build & push is skipped
   (see [Idempotent re-runs](#idempotent-re-runs)).
4. Build & push: `docker/build-push-action@v6` with `tags: <repo>:<version>`,
   `provenance: false` (porter issues its own SLSA attestation instead
   of the buildkit-default one) — **unless** a repo-owned `publish` command
   is set (see below), in which case porter runs that instead.
5. When signing is enabled: `cosign sign` + `cosign attest --type
   slsaprovenance1` against `<repo>@<digest>`. See [Signing](#signing).

### Repo-owned publish command

porter doesn't model build knobs (build args, secrets, target stages,
cache config). Instead, when an image needs them, set a `publish` command and
the repo owns the build:

```toml
# A [registries.ecr] with aws-ecr auth (see Registries) covers login for all
# rows below — including the publish-command rows; the command only owns the
# build, not auth.
[[group]]
name = "default"
components = [
  # api/worker share one Dockerfile, selected by a plain build arg.
  { id = "api", type = "regex", path = "rust/Cargo.toml", pattern = '…',
    artifact = { kind = "oci-image", context = ".", dockerfile = "rust/Dockerfile",
      registry = "ecr",
      publish = "docker buildx build --push --platform $PORTER_PLATFORMS --build-arg BIN=api -f $PORTER_DOCKERFILE -t $PORTER_IMAGE $PORTER_CONTEXT" } },
  { id = "worker", artifact = { kind = "oci-image", context = ".", dockerfile = "rust/Dockerfile",
      registry = "ecr",
      publish = "docker buildx build --push --platform $PORTER_PLATFORMS --build-arg BIN=worker -f $PORTER_DOCKERFILE -t $PORTER_IMAGE $PORTER_CONTEXT" } },
  # web needs a secret build arg — see build-secrets below.
  { id = "web", artifact = { kind = "oci-image", context = ".", dockerfile = "ts/web/Dockerfile",
      registry = "ecr",
      publish = "docker buildx build --push --platform $PORTER_PLATFORMS --build-arg FONTAWESOME_NPM_AUTH_TOKEN=$FONTAWESOME_NPM_AUTH_TOKEN -f $PORTER_DOCKERFILE -t $PORTER_IMAGE $PORTER_CONTEXT" } },
]
```

When `publish` is set, the workflow runs it **instead of**
`docker/build-push-action`. The command must build *and* push to
`$PORTER_IMAGE`; porter then resolves the pushed digest for signing (or reads
it from `$PORTER_DIGEST_FILE` if the command writes one). **Auth is not the
command's job** — porter still logs in per the registry's declared auth kind
(`github-token`/`basic`/`token`/`aws-ecr`) before the command runs, so the
command builds into an authenticated context. To have the command own auth
too, point it at a `none`-auth (bare-URL) registry and log in inside the
command. porter exposes these env vars:

| Var | Value |
| --- | --- |
| `PORTER_IMAGE` | the full ref to push, `<repo>:<version>` |
| `PORTER_VERSION` | bare version, e.g. `0.5.3` |
| `PORTER_TAG` | the component's git tag, e.g. `api/v0.5.3` |
| `PORTER_CONTEXT` | the artifact `context` |
| `PORTER_DOCKERFILE` | the artifact `dockerfile` |
| `PORTER_PLATFORMS` | comma-joined `platforms` |
| `PORTER_REGISTRY` | the resolved image repo |
| `PORTER_DIGEST_FILE` | optional output: write the pushed `sha256:…` here if `imagetools inspect $PORTER_IMAGE` won't resolve it |

**Secret build args.** Pass them via the reusable workflow's
`build-secrets` secret — a JSON object of `name → value`. Each entry is
exported as an env var the `publish` command can reference (`--build-arg
NAME=$NAME` or buildx `--secret id=NAME`). This mirrors `registry-auth`;
values are masked in logs.

```yaml
# in your calling workflow
secrets:
  build-secrets: ${{ toJSON(secrets) }}   # or a curated subset
```

Idempotency, registry login, and signing all work the same for a
repo-owned publish command as for the default build.

## `helm-chart`

Package a chart with `helm package` and push it to an OCI registry.

```toml
[[group]]
name = "charts"
components = [
  { id = "platform", type = "helm-chart", path = "deploy/helm/platform/Chart.yaml",
    artifact = { kind = "helm-chart",
      chart = "deploy/helm/platform",                 # chart directory
      registry = "oci://ghcr.io/tractorbeamai/charts" } },  # OCI registry, or a [registries] name
]
```

**Matrix expansion:** one row per component. Runner: `ubuntu-latest`.

**Workflow steps** (`if: matrix.kind == 'helm-chart'`):
1. Registry login (see [Registries](#registries)).
2. Resolve publish state: check whether `<chart name>:<version>` is already
   in the registry; if so the package & push is skipped (see [Idempotent
   re-runs](#idempotent-re-runs)).
3. Resolve dependencies: a chart with a committed `Chart.lock` runs
   `helm dependency build` (reproducible from the lock); a chart that
   declares `dependencies:` without a lock runs `helm dependency update`
   (resolve + fetch); a dependency-free chart skips this. `helm package`
   won't fetch subcharts itself, so charts with remote dependencies fail
   to package unless they're vendored under `charts/` first.
4. `helm package <chart> --version <version> --app-version <same> -d dist`
   (the matrix `version` is already the bare `X.Y.Z` helm expects).
5. `helm push dist/<chart>-<version>.tgz <registry>` — the pushed ref
   and digest are parsed from helm's output.
6. When signing is enabled: `cosign sign` + `cosign attest --type
   slsaprovenance1` against the pushed `<repo>@<digest>` (a chart in an
   OCI registry is just another OCI artifact). See [Signing](#signing).

**Note:** the same component carries the version source (`type = "helm-chart"`
rewriting `Chart.yaml`) and the artifact — one component, bumped and published
in lockstep. That's porter's whole point.

## Idempotent re-runs

Releases are safe to re-run. Before pushing, the `oci-image` and
`helm-chart` steps query the registry and **skip the push when this
version is already published** — so re-running a partially-failed release
(some artifacts pushed, others not) finishes the remainder instead of
hard-failing. This is mandatory for **IMMUTABLE** registries such as AWS
ECR, where re-pushing an existing tag is a hard error. When the push is
skipped, the published digest is still resolved so signing/attestation
runs (a prior run that pushed but failed to sign completes on re-run).

`cli-binary` uploads already use `gh release upload --clobber`, and the
`tag` job skips tags the remote already has, so those paths are
re-runnable too.

## `npm-package`

Publish a JavaScript package to a registry via `npm publish`.

```toml
[[group]]
name = "sdk"
components = [
  { id = "sdk", type = "package-json", path = "ts/packages/sdk/package.json",
    artifact = { kind = "npm-package",
      path = "ts/packages/sdk",                 # directory containing package.json
      registry = "https://registry.npmjs.org" } },  # default; a [registries] name for a private one
]
```

**Matrix expansion:** one row per component. Runner: `ubuntu-latest`.

**Workflow steps** (`if: matrix.kind == 'npm-package'`):
1. Write a `.npmrc` in `path` that auths against `registry`.
2. `npm publish --access public` from `path`.

**Authentication:** a registry with `token` auth reads its token from the
`registry-auth` JSON secret; the default registry (no declared auth) falls
back to the `npm-token` secret. If neither resolves the step errors loudly
rather than publishing unauthenticated. See [Registries](#registries).

The same component carries the version source (`package-json` rewriting
`package.json`) and the artifact, so its version and publish move together.

## `python-wheel`

Build a Python wheel with `maturin` (PyO3-style native crates) and
upload it to the GitHub Release. PyPI publishing is intentionally
not in scope yet — wheels land on the release page where downstream
infrastructure can pick them up.

```toml
[[group]]
name = "py"
components = [
  { id = "client", type = "regex", path = "py/client/pyproject.toml",
    pattern = '(?m)^version = "(?P<version>[^"]+)"',
    artifact = { kind = "python-wheel", path = "py/client" } },
]
```

**Matrix expansion:** one row per component. Runner: `ubuntu-latest`.

**Workflow steps** (`if: matrix.kind == 'python-wheel'`):
1. `pipx install maturin`
2. `maturin build --release --out ../dist` from `path`.

The wheel ends up in `dist/` and is uploaded with the rest of the
release assets.

## Registries

`oci-image`, `helm-chart`, and `npm-package` publish to a registry. The
`registry` field is either a key into `[registries]` or a bare URL (used as-is,
anonymous):

```toml
[registries.ghcr]
kind = "oci"                       # oci | oci-helm | npm | pypi
url  = "ghcr.io/acme"
auth = { type = "github-token" }   # GITHUB_TOKEN — the common ghcr.io case

[registries.dockerhub]
kind = "oci"
url  = "docker.io/acme"
auth = { type = "basic", username_secret = "DH_USER", password_secret = "DH_PAT" }

[registries.ecr]
kind = "oci"                                                  # oci-helm for charts
url  = "111122223333.dkr.ecr.us-east-1.amazonaws.com/acme"
auth = { type = "aws-ecr", role_arn = "arn:aws:iam::111122223333:role/gha", region = "us-east-1" }
```

`auth.type` is `none`, `github-token`, `basic` (username/password), `token`
(a single bearer token), or `aws-ecr`. `basic`/`token` credentials are
referenced by **secret name**: GitHub Actions can't index the `secrets` context
by a dynamic key, so the release workflow reads them from one `registry-auth`
JSON secret — `{"DH_USER": "...", "DH_PAT": "..."}` — that the caller passes.
`github-token` auth needs no `registry-auth` entry (it uses the workflow's
token).

`aws-ecr` authenticates to AWS ECR via GitHub Actions OIDC — its `role_arn`
and `region` are **plain values, not secret names**. porter runs
`aws-actions/configure-aws-credentials` (assuming `role_arn`) then
`aws ecr get-login-password | docker login` (and `helm registry login` for
chart rows). The caller's job that invokes the reusable workflow must grant
`id-token: write` (already required for cosign signing). Valid only on
`oci`/`oci-helm` registries.

porter validates that a named registry's `kind` matches the artifact that
references it (an `oci-image` can't point at an `npm` registry).

## Mixing kinds in one repo

A polyglot repo declares several components across groups — version lines that
move independently:

```toml
[[group]]
name = "app"
components = [
  { id = "mytool", type = "cargo-workspace", path = "Cargo.toml", tag_prefix = "v",
    artifact = { kind = "cli-binary", package = "mytool-cli",
      targets = ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"] } },
  { id = "api", artifact = { kind = "oci-image", context = ".",
    dockerfile = "Dockerfile", registry = "ghcr.io/example/api" } },
]

[[group]]
name = "charts"
components = [
  { id = "platform", type = "helm-chart", path = "deploy/helm/platform/Chart.yaml",
    artifact = { kind = "helm-chart", chart = "deploy/helm/platform",
      registry = "oci://ghcr.io/example/charts" } },
]
```

The matrix expands to four rows (two `cli-binary` targets + one `oci-image` +
one `helm-chart`); each lands on its appropriate runner and runs its
kind-specific step block, in parallel (`fail-fast: false`). The `app` and
`charts` groups version and tag independently.

## Extending with a new kind

Adding a new artifact kind is three changes:

1. New variant in `Artifact` ([crates/porter-core/src/config.rs](../crates/porter-core/src/config.rs)).
2. New match arm in `build_matrix` ([crates/porter-core/src/matrix.rs](../crates/porter-core/src/matrix.rs)).
3. New step block in [release.yml](../.github/workflows/release.yml)
   gated on `matrix.kind == '<new-kind>'`.

The matrix row carries arbitrary metadata via the kind-specific
`Option<...>` fields on `MatrixRow`; if your new kind needs a field
no other kind has, add it there. The fields are flattened to JSON so
the workflow accesses them as `matrix.<field>`.
