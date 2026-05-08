// Restriction-group lints that are universally exempted from test code:
// tests are allowed to panic on `Result::Err`, slice into known-good
// vectors, do unchecked arithmetic, and bind throwaway results. These
// conflict with the workspace's `clippy::restriction = warn` policy
// only inside `#[cfg(test)]` blocks; production code remains under the
// strict policy.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic_in_result_fn,
        clippy::str_to_string,
        clippy::missing_panics_doc,
        clippy::missing_errors_doc,
    )
)]

//! porter-core — release-cutting machinery for polyglot monorepos.
//!
//! This is the library half of [porter]; the
//! [`porter-cli`](https://github.com/tractorbeamai/porter/tree/main/crates/porter-cli)
//! crate consumes it. Most users will interact with the CLI rather
//! than this crate directly; the library is published primarily so
//! the CLI's behavior is testable and reusable from other Rust
//! tools.
//!
//! # Layers
//!
//! The crate is split into independently testable layers:
//!
//! - [`changeset`] — parse and write `.changeset/*.md` files (semver bump +
//!   summary, frontmatter compatible with `knope-dev/changesets`).
//! - [`config`] — load and validate `porter.toml`.
//! - [`versioned_files`] — adapters that read and rewrite the version
//!   string embedded in concrete file formats (Cargo workspace, Helm
//!   chart, package.json, generic regex).
//! - [`version`] — compute the next semver from a changeset set, honoring
//!   the cargo / Changesets pre-1.0 convention (`0.5.2` + minor → `0.5.3`,
//!   `0.5.2` + major → `0.6.0`).
//! - [`changelog`] — render Markdown sections and prepend them to a
//!   `CHANGELOG.md`.
//! - [`apply`] — orchestrate the whole "version" step: read current,
//!   compute next, rewrite every file, prepend changelog, consume
//!   changesets.
//! - [`build`] — cross-compile a CLI binary, tar-gzip it, and write a
//!   BSD-format SHA-256 line into `dist/checksums.txt`.
//! - [`matrix`] — fan `[[artifacts]]` entries out into a GitHub Actions
//!   `strategy.matrix.include` array.
//! - [`attest`] — build an unsigned in-toto v1 Statement with SLSA Build
//!   Provenance v1 as the predicate (Phase D scaffolding; signing wraps
//!   this output in a DSSE envelope via `cosign`).
//!
//! # Example
//!
//! Compute the next version a `porter version` invocation would produce,
//! without writing anything:
//!
//! ```no_run
//! use porter_core::{Config, ChangesetSet, current_version, compute_next_version};
//! use std::path::Path;
//!
//! # fn main() -> anyhow::Result<()> {
//! let root = Path::new(".");
//! let config_path = Config::discover(root)
//!     .ok_or_else(|| anyhow::anyhow!("no porter.toml found"))?;
//! let config = Config::load(&config_path)?;
//! let set = ChangesetSet::load_from_dir(&root.join(&config.changesets.directory))?;
//! let current = current_version(root, &config)?;
//! if let Some(next) = compute_next_version(&current, &set)? {
//!     println!("{} -> {}", next.previous, next.next);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Stability
//!
//! Public items re-exported at the crate root form the supported
//! surface; module-level items not re-exported here may move between
//! minor versions. Pin to a specific `0.x.y` if your code depends on
//! the internal layout.
//!
//! [porter]: https://github.com/tractorbeamai/porter

pub mod apply;
pub mod attest;
pub mod build;
pub mod changelog;
pub mod changeset;
pub mod config;
pub mod matrix;
pub mod slug;
pub mod version;
pub mod versioned_files;

pub use apply::{ApplyResult, apply_next_version, current_version};
pub use attest::{AttestInput, Statement, build_statement, sha256_hex};
pub use build::{BuildArtifact, BuildOpts, append_checksum, build_cli_binary};
pub use changelog::{prepend_section, render_section, today_utc};
pub use changeset::{Bump, Changeset, ChangesetSet, write_changeset};
pub use config::{ArtifactConfig, ChangesetMode, Config, SigningBackend, VersionedFileSpec};
pub use matrix::{MatrixRow, build_matrix, render_for_actions};
pub use slug::slugify;
pub use version::{NextVersion, compute_next_version};
pub use versioned_files::{VersionedFile, load_versioned_file};
