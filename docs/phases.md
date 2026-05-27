# Phase plan

porter is delivered in five phases. Commit messages and code comments
reference them ("Phase B implements the artifact pipeline...", "Phase D
scaffolding..."), so this page is the single source of truth for what
each phase covers and what's done.

## Phase A — core scaffold

**Status:** ✓ shipped.

The `porter-core` library and `porter-cli` binary, with end-to-end
support for the version-bumping loop:

- `porter add` writes a changeset.
- `porter status` reports current/next.
- `porter version` rewrites each bumped group's version sources,
  prepends its changelog, consumes the changesets.
- `porter release tag` / `porter release notes` query the rendered
  changelog.

Version-source kinds: `cargo-workspace`, `helm-chart`, `package-json`,
generic `regex`. All four are unit-tested against the obvious failure
modes (drift detection, missing fields, formatting preservation).

## Phase B — artifact pipeline + self-release

**Status:** ✓ shipped (cli-binary fully end-to-end; other kinds are
workflow-step skeletons).

The release pipeline: `porter matrix` emits a GitHub Actions job
matrix from each group's artifact-bearing components; the reusable
`release.yml` consumes it and dispatches per-kind step blocks. `porter build cli-binary`
cross-compiles, archives, and writes a BSD-format SHA-256 line into
`dist/checksums.txt`.

`setup-porter/action.yml` consumes the published checksum file to
verify downloads. porter ships itself via this exact path —
`porter-release.yml` is the self-bootstrap workflow that drives the
matrix using a host-built porter to cross-compile per-target porter
binaries.

The `oci-image`, `helm-chart`, `npm-package`, and `python-wheel`
kinds have matrix expansion + workflow step blocks but haven't been
exercised by a real consumer yet. See
[`artifact-kinds.md`](artifact-kinds.md) for the implementation
status table.

## Phase C — the App and the ruleset

**Status:** ✓ shipped. The porter App is registered in the
`tractorbeamai` org, installed on `tractorbeamai/porter`, and the
ruleset is active and exercised — v0.1.0 was the first release that
went through this loop end-to-end.

What "porter is the sole privileged tagger" means in practice:

- A dedicated GitHub App (spec'd in [`app/spec.yml`](../app/spec.yml))
  whose installation token is the only identity allowed to push tags
  matching `refs/tags/v*`.
- A repository ruleset ([`tools/install-ruleset.sh`](../tools/install-ruleset.sh))
  that names that App as the sole `bypass_actor` and rejects every
  other identity.

Until the ruleset is installed, porter is just a release author;
humans can `git tag && git push` to bypass it. After installation,
that push fails with a ruleset-violation message — the verification
step described in [`app/README.md`](../app/README.md).

## Phase D — attestation

**Status:** ⏳ implemented across all signable kinds; not yet exercised
end-to-end by a real consumer.

Signing is config-driven via `[signing]` (opt-in: add the block to turn
on keyless Sigstore) and uniform across artifact kinds. `porter matrix`
stamps each signable row; `release.yml` signs and attaches SLSA Build
Provenance v1:

- `oci-image` and `helm-chart`: `cosign sign` + `cosign attest --type
  slsaprovenance1` by registry digest.
- `cli-binary`: `cosign sign-blob` + `cosign attest-blob`, detached
  bundles uploaded to the Release.

porter owns the *predicate* (`porter attest --emit predicate`) — build
identity, source, invocation — and cosign owns the *subject*, computing
the digest from the artifact it signs and building the in-toto Statement.
porter's own self-release is wired the same way (gated on porter.toml's
`[signing]`), ready to dogfood the moment signing is switched on.

Consumers verify the chain with `cosign verify-attestation` (images) or
`cosign verify-blob-attestation` (binaries) against the porter App's
identity. The Phase C ruleset is the cryptographic anchor: because only
the App can mint a release, the App's identity in the attestation is
meaningful.

What's left:
- Sign + verify roundtrip exercised in CI against a real consumer.
- `cosign verify-attestation` integration baked into `setup-porter`
  or a sibling action so consumers don't need to wire the verification
  themselves.
- A first-class registry-login hook for non-GHCR registries (ECR, Docker
  Hub) so OCI signing works without a consumer-side login step.

## Phase E — admission policy

**Status:** ⏳ template only.

Once attestations are flowing, the next step is gating *deployment*
on attestation provenance. [`policy/cluster-image-policy.example.yaml`](../policy/cluster-image-policy.example.yaml)
sketches a Kyverno or Sigstore Policy Controller policy that admits
container images only if their attestations chain back to the porter
App's identity.

This is intentionally low-priority until Phase D is exercised. A
policy is only as useful as the signal it gates on; we want
attestations to be reliable before downstream systems start failing
deployments based on them.

## What's *not* in scope

- **PyPI publishing.** `python-wheel` artifacts land on the GitHub
  Release page. Pushing to PyPI is a separate concern; consumers can
  add a step that uploads from there.
- **Crates.io publishing.** Same as above — `cargo publish` can be a
  consumer-side step that runs after the release tag is created.
- **Mirror replication.** No "publish to multiple registries"
  fan-out; each component publishes one artifact to one registry.
- **Pre-releases / alphas / betas.** porter computes one canonical
  next version per group. Pre-release identifiers (`-alpha.1`,
  `-rc.2`) aren't modeled.
- **Linked versioning.** A group pins its members to one shared
  version (changesets' "fixed" behavior). "Linked" mode — members
  bumped together but allowed to drift — isn't modeled.

## Multi-line releases (built)

porter is no longer single-mode. A repo declares one or more `[[group]]`
blocks; each group is an independent version line with its own changelog
and its own tags, and a changeset names the group(s) it bumps. A
component within a group bundles a version source and an optional
artifact, and cuts a per-component tag (`<id>/v<version>`). See the
README "Configuration" section and [artifact-kinds.md](artifact-kinds.md).
