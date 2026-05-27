# Changelog

## 0.2.0 — 2026-05-27

### Breaking changes

- Replace the flat `versioned_files`/`artifacts` config with `[[group]]` blocks of unified components: each group is an independent version line, and a component bundles a version source and an optional artifact. Tags are now per-component (`<id>/v…`). Add a `[registries]` table so artifacts can publish to arbitrary registries with declared auth (github-token/basic/token).

### Features

- Add an `aws-ecr` registry auth kind. A `[registries.*]` entry can declare `auth = { type = "aws-ecr", role_arn = "…", region = "…" }`; the release workflow assumes the role via GitHub OIDC (`aws-actions/configure-aws-credentials`) and logs in with `aws ecr get-login-password` (plus `helm registry login` for chart rows). Unlike `basic`/`token`, `role_arn`/`region` are plain config values, not secret names. Valid only on `oci`/`oci-helm` registries. This lets porter push to AWS ECR on the default build path without the consumer injecting login steps.
- Add a repo-owned `publish` command for the `oci-image` artifact. When set, the release workflow runs it INSTEAD of `docker/build-push-action`, exposing the image ref and build context as `PORTER_*` env vars; the repo owns build args, secrets, and stages, and the command builds and pushes to `$PORTER_IMAGE` (or writes the digest to `$PORTER_DIGEST_FILE`). porter still logs in per the registry's auth kind and signs the pushed digest. Secret build args are supplied through a new `build-secrets` reusable-workflow secret (a JSON `name → value` map, exported as env vars). This unblocks images that need build args — e.g. a shared Dockerfile selected by `--build-arg BIN=…`, or a secret token build arg — without porter modelling each knob.
- Emit a structured publish manifest of what each release shipped, instead of scraping a tool's stdout. New `porter release record` writes one JSON record per artifact (kind, name, group, tag, version, registry, digest / target / sha256), emitted by every build row; `porter release manifest` merges them into a sorted `published.json` the release workflow uploads per release and summarizes. Downstream consumers — Release bodies, notifications, and Phase D attestation — read exact artifact identities and digests from the manifest.

### Fixes

- Resolve a Helm chart's dependencies before packaging it. The reusable release workflow now runs `helm dependency build` (when a `Chart.lock` is committed) or `helm dependency update` (when dependencies are declared without a lock) ahead of `helm package`, so charts with remote subcharts package successfully instead of failing. Dependency-free charts are unaffected.
- Make `oci-image` and `helm-chart` publishing idempotent. The reusable release workflow now checks the registry before pushing and skips the build/push when this version is already published, so re-running a partially-failed release finishes the remainder instead of hard-failing. This is required for IMMUTABLE registries (e.g. AWS ECR), where re-pushing an existing tag is an error. The published digest is resolved either way, so signing still runs against an already-published artifact.

## 0.1.1 — 2026-05-24

### Features

- Add `[release].version_pr_title` to configure the rolling Version PR title (and its commit subject). Supports `{version}` and `{tag}` placeholders — set it to e.g. `chore(release): {version}` for a Conventional Commits subject on the squash-merged commit. `porter status --json` now emits the rendered `pr_title`, and the reusable `version.yml` uses it (its `title` input remains an optional override).
- Add opt-in artifact signing. Declaring a `[signing]` block in `porter.toml` signs every published container image, Helm chart, and CLI binary with cosign (keyless Sigstore) and attaches SLSA build provenance. Images and charts are signed by registry digest; binaries get detached signature and attestation bundles on the release. Without the block, releases are unsigned.

## 0.1.0 — 2026-05-12

### Breaking changes

- Initial release: porter-core, CLI subcommands (add, status, version, release tag/notes), and built-in versioned-file kinds (cargo-workspace, helm-chart, package-json, regex).

