---
bump: patch
---

Make `oci-image` and `helm-chart` publishing idempotent. The reusable release workflow now checks the registry before pushing and skips the build/push when this version is already published, so re-running a partially-failed release finishes the remainder instead of hard-failing. This is required for IMMUTABLE registries (e.g. AWS ECR), where re-pushing an existing tag is an error. The published digest is resolved either way, so signing still runs against an already-published artifact.
