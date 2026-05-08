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
- `porter version` rewrites every `[[versioned_files]]` entry,
  prepends the changelog, consumes the changesets.
- `porter release tag` / `porter release notes` query the rendered
  changelog.

Versioned-file kinds: `cargo-workspace`, `helm-chart`, `package-json`,
generic `regex`. All four are unit-tested against the obvious failure
modes (drift detection, missing fields, formatting preservation).

## Phase B — artifact pipeline + self-release

**Status:** ✓ shipped (cli-binary fully end-to-end; other kinds are
workflow-step skeletons).

The release pipeline: `porter matrix` emits a GitHub Actions job
matrix from `[[artifacts]]`; the reusable `release.yml` consumes it
and dispatches per-kind step blocks. `porter build cli-binary`
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

**Status:** ⏳ partial. App manifest + install instructions shipped;
ruleset installer script shipped; not yet exercised against a real
repo end-to-end.

What "porter is the sole privileged tagger" means in practice:

- A dedicated GitHub App ([`app/manifest.yml`](../app/manifest.yml))
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

**Status:** ⏳ scaffolded, signing wired in CI but not yet exercised
end-to-end.

Each cli-binary release produces an in-toto v1 Statement with SLSA
Build Provenance v1 as the predicate. `porter attest` emits the
*unsigned* statement as JSON; the signing step in `release.yml` pipes
it through `cosign attest-blob` to wrap it in a DSSE envelope and
sign with Sigstore (Fulcio + Rekor) using the workflow's OIDC token.

Consumers verify the chain with `cosign verify-attestation` against
the porter App's identity. The Phase C ruleset is the cryptographic
anchor: because only the App can mint a release, the App's identity
in the attestation is meaningful.

What's left:
- Sign + verify roundtrip exercised in CI.
- `cosign verify-attestation` integration baked into `setup-porter`
  or a sibling action so consumers don't need to wire the verification
  themselves.
- Attestation kinds beyond cli-binary (oci-image gets DSSE-wrapped
  buildkit attestations; helm-chart needs its own predicate).

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
  fan-out; `[[artifacts]]` is one entry per published artifact.
- **Pre-releases / alphas / betas.** porter computes one canonical
  next version. Pre-release identifiers (`-alpha.1`, `-rc.2`) aren't
  modeled.
- **Multi-tenant releases.** porter is single-mode (`changesets.mode
  = "single"`): everything moves at one version. Independent-mode
  (per-package versions) is reserved as a future extension and
  hinted at by `ChangesetMode::Independent` but not implemented.
