---
bump: minor
---

Add opt-in artifact signing. Declaring a `[signing]` block in `porter.toml` signs every published container image, Helm chart, and CLI binary with cosign (keyless Sigstore) and attaches SLSA build provenance. Images and charts are signed by registry digest; binaries get detached signature and attestation bundles on the release. Without the block, releases are unsigned.
