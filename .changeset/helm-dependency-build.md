---
bump: patch
---

Resolve a Helm chart's dependencies before packaging it. The reusable release workflow now runs `helm dependency build` (when a `Chart.lock` is committed) or `helm dependency update` (when dependencies are declared without a lock) ahead of `helm package`, so charts with remote subcharts package successfully instead of failing. Dependency-free charts are unaffected.
