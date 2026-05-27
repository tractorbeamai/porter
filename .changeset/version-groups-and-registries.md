---
bump: major
---

Replace the flat `versioned_files`/`artifacts` config with `[[group]]` blocks of unified components: each group is an independent version line, and a component bundles a version source and an optional artifact. Tags are now per-component (`<id>/v…`). Add a `[registries]` table so artifacts can publish to arbitrary registries with declared auth (github-token/basic/token).
