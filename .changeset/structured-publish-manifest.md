---
bump: minor
---

Emit a structured publish manifest of what each release shipped, instead of scraping a tool's stdout. New `porter release record` writes one JSON record per artifact (kind, name, group, tag, version, registry, digest / target / sha256), emitted by every build row; `porter release manifest` merges them into a sorted `published.json` the release workflow uploads per release and summarizes. Downstream consumers — Release bodies, notifications, and Phase D attestation — read exact artifact identities and digests from the manifest.
