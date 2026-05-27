---
bump: minor
---

Add an `aws-ecr` registry auth kind. A `[registries.*]` entry can declare `auth = { type = "aws-ecr", role_arn = "…", region = "…" }`; the release workflow assumes the role via GitHub OIDC (`aws-actions/configure-aws-credentials`) and logs in with `aws ecr get-login-password` (plus `helm registry login` for chart rows). Unlike `basic`/`token`, `role_arn`/`region` are plain config values, not secret names. Valid only on `oci`/`oci-helm` registries. This lets porter push to AWS ECR on the default build path without the consumer injecting login steps.
