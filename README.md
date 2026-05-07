# porter

Release-cutting tool for polyglot monorepos. Atomically bumps every version-bearing file (Cargo workspaces, Helm charts, package.jsons, Terraform pins) from a single `vX.Y.Z`, then drives matrix builds, Sigstore-signed in-toto attestations, and GitHub Releases. Designed to be the sole privileged tagger for its host repo.
