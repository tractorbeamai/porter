---
bump: minor
---

Add composite actions `porter-tag`, `porter-sign`, and `porter-manifest` so a repo can own its build and run cosign in its own job — signing release artifacts under its own identity rather than porter's. The admission-policy example now pins an org-wide signing subject and leans on the SLSA predicate (`source`, `builder`) for the per-repo and release-tag guarantees. See the new `docs/signing-and-trust.md`.
