# Contributing

porter is small, opinionated, and aggressively self-hosted: it cuts its
own releases through the same code paths consumer repos exercise.
Patches that don't pay back the maintenance cost of porter's own
release loop will be rejected, even if they're individually fine.

## Local development

```sh
# clone
gh repo clone tractorbeamai/porter
cd porter

# build everything
cargo build --workspace

# run all unit + integration tests
cargo test --workspace

# enforce the workspace's strict lint policy (don't ship without it)
cargo clippy --workspace --all-targets -- -D warnings

# format
cargo fmt --all
```

The workspace lint policy in [`Cargo.toml`](Cargo.toml) opts in to
clippy's `pedantic`, `nursery`, `cargo`, and `restriction` groups, then
allows individual lints with a documented rationale. **Adding a new
`#[allow]` requires a comment explaining why** — see existing examples
for the form. If clippy fires on a real bug, fix the bug; if it fires
on stylistic noise, allow it at the workspace root with a comment, not
inline.

## Test layout

- **Unit tests** live in each module's `#[cfg(test)] mod tests` block.
  The convention is `verb_condition` naming (`reads_version`,
  `writes_preserves_v_prefix`) and `tempfile::TempDir` + `indoc!` for
  fixtures. See `crates/porter-core/src/versioned_files/cargo_workspace.rs`
  for the canonical shape.
- **Integration tests** live in `crates/porter-cli/tests/*.rs`. They
  build the binary via `env!("CARGO_BIN_EXE_porter")` and exercise it
  against a tempdir fixture. No new dev-deps required for these — Cargo
  exposes the binary path automatically.

When you add a new behavior, add a unit test in the most local module
that covers it. When you fix a bug, add a test that fails before your
fix and passes after — `git stash` the fix, run the test, confirm it
fails, then unstash.

## Dogfooding porter on porter

The whole repo is configured to release itself with porter. The flow
when working on a release-worthy change:

1. Make your change.
2. `porter add --bump <patch|minor|major> --summary "<one line>"` —
   commit the resulting `.changeset/*.md` alongside your change.
3. `porter version --dry-run` — sanity-check what the next release
   would do. Should report `<current> -> <next>` and list every
   versioned file it would rewrite.
4. Open the PR.

When the PR merges to `main`, the rolling Version PR updates. When
*that* merges, the release fires.

If you're touching the version-rewriting code, run `porter version
--dry-run` against a fixture repo (or just porter itself) to make
sure the diff is exactly what you intended. The dry-run is the
cheapest acceptance test for that whole code path.

## Pull request expectations

- **One concern per PR.** Mixing a bug fix with a refactor and a
  feature in one PR makes the review job 3× the work.
- **Tests with the change.** New behavior gets a test. Bug fixes get a
  test that fails before the fix and passes after.
- **Commit messages over PR descriptions.** The repo prefers a clean
  commit log over a long PR description that gets squashed away.
  Imperative-mood subject under 70 chars; body wrapped at 72 columns
  explaining *why*, not just *what*.
- **Conventional-commit prefixes welcome but optional.** `feat:`,
  `fix:`, `docs:`, `test:`, `chore:`, `ci:` are all in use.
- **Squash by default**, rebase merge if your commit history is
  meticulous and worth keeping.

## Lint policy notes

The workspace lint policy is strict by design. A few things worth
knowing if you bump into it:

- `clippy::expect_used` and `clippy::unwrap_used` are warn-level. In
  production code, swap to `?` or document the panic with a
  `# Panics` rustdoc block and an `#[expect(clippy::expect_used,
  reason = "...")]` attribute. See `attest.rs` for the form.
- `clippy::missing_errors_doc` and `clippy::missing_panics_doc` apply
  to public functions returning `Result` / functions that can panic.
  Add the appropriate rustdoc section.
- Test code is exempt from the noisier lints via the `#![cfg_attr(test,
  allow(...))]` blocks at the top of `lib.rs` and `main.rs`.
  Integration tests are their own crate and need their own `#![allow]`
  block (see `tests/cli_dry_run.rs`).

## What to expect from review

Expect a thorough read. The review process for a release-cutting tool
is unforgiving: a wrong rewrite in any of the version-bearing file
adapters can corrupt manifests across every porter consumer. Reviews
will pull on edge cases, and "I'll add a test in a follow-up" doesn't
fly for the hot paths (the four `versioned_files/*` adapters, the
`apply.rs` orchestrator, the `attest.rs` provenance builder).

## Reporting bugs

Open an issue. Include:
- The version of porter (`porter --version`) and the platform.
- The relevant slice of `porter.toml`.
- The exact command and its output.
- For version-bumping bugs: the file's contents before, the expected
  contents after, and what porter actually wrote.

For security-relevant bugs (anything that could let a non-App identity
mint a tag, forge an attestation, or trick `setup-porter` into
installing an unverified binary), follow [`SECURITY.md`](SECURITY.md)
instead of opening a public issue.
