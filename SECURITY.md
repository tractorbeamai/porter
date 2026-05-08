# Security policy

porter cuts releases for the repos it's installed in. A vulnerability
in porter is, in the worst case, a vulnerability in every release every
porter-managed repo has ever shipped — bugs here have outsized blast
radius. We treat security reports accordingly.

## Reporting a vulnerability

**Don't open a public issue.** Email `security@tractorbeam.ai` with:

- A description of the vulnerability and its impact.
- The minimum reproducer you have. A working PoC is appreciated but
  not required.
- The version(s) of porter affected, if you know.
- Any constraints on disclosure timing on your end (e.g. coordinated
  disclosure with another upstream).

We acknowledge reports within two business days. If you don't hear
back in that window, assume the email got lost and ping again on a
different channel (e.g. a private DM on the relevant Tractorbeam Slack
or a direct message to a maintainer on GitHub).

## What we consider in scope

The high-impact surfaces, ordered by blast radius:

- **Anything that lets a non-App identity mint a `v*` tag** in a
  porter-managed repo. The whole point of the App + ruleset is that
  releases originate from one identity; a bypass here invalidates
  every downstream attestation.
- **Anything that lets an attacker forge an attestation** that
  `cosign verify-attestation` will accept against the porter App's
  identity. This includes both signing-side bugs (Phase D scaffolding
  in `attest.rs`) and verification-side bugs (the `setup-porter`
  consumer side, once Phase D ships verification).
- **Supply-chain bugs in `setup-porter`** that could install a binary
  whose checksum doesn't match the release-published `checksums.txt`.
  The known-good case is documented in
  [`actions/setup-porter/action.yml`](actions/setup-porter/action.yml);
  any deviation that successfully installs is in scope.
- **Manifest-corruption bugs** in the four `versioned_files/*`
  adapters that could cause porter to write a malformed file under
  attacker-controlled input (e.g. a maliciously-crafted `Chart.yaml`,
  `package.json`, or `porter.toml` in a PR that, when parsed by
  porter, causes a write that destroys repository state). These have
  smaller blast radius than the above but still matter.
- **Privilege escalation in the reusable workflows.** A consumer
  using `tractorbeamai/porter/.github/workflows/release.yml@v0`
  passes secrets in via `secrets:`; a bug that lets a workflow leak
  those secrets to a non-bypass identity is in scope.

## What we consider out of scope

- **Denial of service via pathological regex patterns** in user-controlled
  `porter.toml`. The `regex` crate is linear-time, so this can't be a
  ReDoS, but a pattern that matches the wrong bytes is the user's
  responsibility — porter documents that the named group should match
  the version token and nothing else.
- **Markdown injection in changeset summaries.** Changesets are
  written by PR authors and rendered as Markdown on the public
  GitHub Release page; the trust boundary is clearly documented in
  `crates/porter-core/src/changelog.rs`. Reviewers should review
  changeset content like any other content landing in the repo.
- **Issues that require a compromised maintainer account or runner.**
  If the attacker can already push to `main`, they don't need a
  porter vulnerability to ship a backdoored release.

## Coordinated disclosure

We aim for fix-and-disclose within 30 days for high-severity issues
and 90 days for medium-severity. We'll credit reporters in the
release notes unless you'd rather not be named. If the issue requires
upstream coordination (e.g. a Sigstore or `time` crate bug), we'll
work the timeline against their disclosure window.

## Security-relevant releases

Security-relevant releases are tagged like any other and get a
changelog entry under `### Fixes` with a `(security)` suffix. We
publish a corresponding GitHub Security Advisory with the CVE if one
is assigned. There's no separate `vX.Y.Z+sec` channel; consumers on
the rolling-tag (`@v0`) get the fix automatically, consumers on a
pinned tag will need to bump.
