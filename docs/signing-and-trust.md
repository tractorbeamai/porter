# Signing & trust model

porter signs release artifacts with [cosign] keyless and attaches a [SLSA
Build Provenance v1] attestation, so a cluster admission policy (Sigstore
[policy-controller]) can refuse any image that didn't come out of a trusted
release. This doc explains *what* the signature proves, *who* holds the
signing identity, and the two ways to wire it up.

## porter is a capability layer, not a job

porter owns the "what to release" concerns — versioning, the changelog,
cutting tags, the SLSA predicate (`porter attest`), and the release manifest.
It does **not** own how you build, how you authenticate to a registry, or how
you handle build secrets. Those are yours to compose.

That split is exposed as composite actions you assemble in your own workflow:

| Action | Job | Does |
| --- | --- | --- |
| [`porter-tag`](../actions/porter-tag) | runs once | cut the protected component tags (`porter[bot]`), open a Release per tag, emit the build matrix |
| [`porter-sign`](../actions/porter-sign) | per artifact | `cosign sign` + attach porter's SLSA predicate, and emit the publish record |
| [`porter-manifest`](../actions/porter-manifest) | runs once | merge per-row records into `published.json` + the canonical `checksums.txt` |

The [reusable `release.yml`](../.github/workflows/release.yml) is the
**batteries-included** path: it stitches the same steps together and owns the
build for you (including the `build-secrets` bag). Use it when you don't need
to own the build. Use the actions when you do.

## The trust model — three checks, each doing one job

Keyless cosign produces a short-lived Fulcio certificate whose **subject** is
the identity of the workflow that ran `cosign`, and logs the signature in
[Rekor]. porter additionally attaches a SLSA predicate. An admission policy
checks three independent things — see
[`policy/cluster-image-policy.example.yaml`](../policy/cluster-image-policy.example.yaml):

1. **Subject (SAN) → *who* signed.** Pinned to an org-wide regex,
   `…/tractorbeamai/<any-repo>/.github/workflows/release.yml@…`. Unforgeable
   (GitHub OIDC). It is deliberately **ref-agnostic** — it does not try to
   encode "released from a tag".
2. **Predicate `source` → *what* was built, and *from where*.**
   `<repo>@refs/tags/v…`, pinned **per image glob** in the policy. Trustworthy
   *because* a trusted workflow (per the subject) signed it. The per-glob pin
   is what stops a *different* org repo from signing an image that claims to
   be this one.
3. **Predicate `builder` → *how*.** `https://github.com/tractorbeamai/porter`
   — porter tooling issued the provenance, regardless of which repo ran it.

### Why the subject doesn't carry the tag

It's tempting to want `@refs/tags/v…` in the cert subject as the "released
from a tag" proof. Resist it. That form only ever appeared because the old
reusable workflow was pinned to a porter version tag, so the subject
*inherited* `@v0.2.0` for free — an accident, not a guarantee. The real
release-tag integrity lives upstream, in each publishing repo:

> **Operational obligation:** every repo whose images a policy admits **must**
> protect its default branch and gate `v…` tag pushes to the porter App
> (`porter-tag` pushes as `porter[bot]`; back it with a tag ruleset). Only
> then is the predicate's `source = …@refs/tags/v…` something you can trust.
> The org-wide subject no longer pins one audited pipeline, so this protection
> can't be skipped in any publishing repo.

## Two consumer shapes

`porter-sign` is **trigger-agnostic** — it signs whatever digest you hand it,
in whatever job calls it. Which job you call it from is a defense-in-depth
dial, not something porter bakes in.

- **Sign on `main` (default, simplest).** One workflow on push-to-main cuts
  tags, builds, and signs in the same run. The subject is
  `…/<repo>/release.yml@refs/heads/main`. The release-tag binding rests on the
  predicate `source` + branch protection + App-gated tags. This matches the
  ergonomics of the reusable workflow.
- **Sign on tag push (hardened).** Split into two workflows — one on `main`
  cuts tags (`porter-tag`), one on `push: tags` builds + signs. The subject
  becomes `@refs/tags/v…`, closing the narrow window where a bad commit merged
  to `main` could auto-sign. Costs you a second workflow (and per-tag matrix
  filtering when a release cuts many component tags). porter's own
  `porter-release.yml` works this way.

You can start on `main` and harden later with **no porter changes** — only
your workflow's trigger moves.

## Owning your build (the `porter-sign` path)

When you need custom build args, build secrets, or multi-stage Dockerfiles,
own the build in your own job and let porter sign the result. Sketch:

```yaml
permissions:
  contents: write   # tag push + Release uploads
  id-token: write   # keyless OIDC — signs under THIS repo's identity
jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@<sha>      # fetch-depth: 0, persist-credentials: false
        with: { fetch-depth: 0, persist-credentials: false }
      - uses: tractorbeamai/porter/actions/setup-porter@<sha>
      - id: tag
        uses: tractorbeamai/porter/actions/porter-tag@<sha>
        with:
          app-id: ${{ secrets.PORTER_APP_ID }}
          app-private-key: ${{ secrets.PORTER_APP_PRIVATE_KEY }}

      # Your build — native. Secrets are yours; nothing crosses into porter.
      - uses: sigstore/cosign-installer@<sha>
      - run: |
          docker buildx build --push \
            --secret id=npm,env=NPM_TOKEN \
            -t "$IMAGE" .
        env:
          NPM_TOKEN: ${{ secrets.NPM_TOKEN }}
          IMAGE: 575108936009.dkr.ecr.us-east-1.amazonaws.com/api:${{ steps.x.outputs.version }}

      - uses: tractorbeamai/porter/actions/porter-sign@<sha>
        with:
          kind: oci-image
          name: api
          group: default
          tag: api/v1.2.3
          version: 1.2.3
          id: oci-api
          registry: 575108936009.dkr.ecr.us-east-1.amazonaws.com/api
          ref: 575108936009.dkr.ecr.us-east-1.amazonaws.com/api@sha256:…

      - uses: tractorbeamai/porter/actions/porter-manifest@<sha>
        with: { tags: ${{ steps.tag.outputs.tags }} }
```

`porter-sign` requires `porter` (via `setup-porter`) and `cosign` on PATH, and
`id-token: write` + `contents: write` in the job. Unsigned kinds
(`npm-package`, `python-wheel`) call it with `sign: 'false'` to record only.

[cosign]: https://docs.sigstore.dev/cosign/overview/
[SLSA Build Provenance v1]: https://slsa.dev/spec/v1.0/provenance
[policy-controller]: https://docs.sigstore.dev/policy-controller/overview/
[Rekor]: https://docs.sigstore.dev/logging/overview/
