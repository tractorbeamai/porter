# Changelog

## 0.1.1 — 2026-05-24

### Features

- Add `[release].version_pr_title` to configure the rolling Version PR title (and its commit subject). Supports `{version}` and `{tag}` placeholders — set it to e.g. `chore(release): {version}` for a Conventional Commits subject on the squash-merged commit. `porter status --json` now emits the rendered `pr_title`, and the reusable `version.yml` uses it (its `title` input remains an optional override).
- Add opt-in artifact signing. Declaring a `[signing]` block in `porter.toml` signs every published container image, Helm chart, and CLI binary with cosign (keyless Sigstore) and attaches SLSA build provenance. Images and charts are signed by registry digest; binaries get detached signature and attestation bundles on the release. Without the block, releases are unsigned.

## 0.1.0 — 2026-05-12

### Breaking changes

- Initial release: porter-core, CLI subcommands (add, status, version, release tag/notes), and built-in versioned-file kinds (cargo-workspace, helm-chart, package-json, regex).

