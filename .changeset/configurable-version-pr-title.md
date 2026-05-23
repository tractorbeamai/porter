---
bump: minor
---

Add `[release].version_pr_title` to configure the rolling Version PR title (and its commit subject). Supports `{version}` and `{tag}` placeholders — set it to e.g. `chore(release): {version}` for a Conventional Commits subject on the squash-merged commit. `porter status --json` now emits the rendered `pr_title`, and the reusable `version.yml` uses it (its `title` input remains an optional override).
