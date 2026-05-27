---
bump: minor
---

Add a repo-owned `publish` command for the `oci-image` artifact. When set, the release workflow runs it INSTEAD of `docker/build-push-action`, exposing the image ref and build context as `PORTER_*` env vars; the repo owns build args, secrets, and stages, and the command builds and pushes to `$PORTER_IMAGE` (or writes the digest to `$PORTER_DIGEST_FILE`). porter still logs in per the registry's auth kind and signs the pushed digest. Secret build args are supplied through a new `build-secrets` reusable-workflow secret (a JSON `name → value` map, exported as env vars). This unblocks images that need build args — e.g. a shared Dockerfile selected by `--build-arg BIN=…`, or a secret token build arg — without porter modelling each knob.
