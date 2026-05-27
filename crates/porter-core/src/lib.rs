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
//! - [`config`] — load and validate `porter.toml` (groups of unified
//!   components).
//! - [`groups`] — domain helpers over the group/component model: group
//!   lookup, a group's version sources and artifacts, and component tags.
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
//! - [`matrix`] — fan each group's artifact-bearing components out into a
//!   GitHub Actions `strategy.matrix.include` array.
//! - [`attest`] — build SLSA Build Provenance v1, either as a bare
//!   predicate (for `cosign attest`/`attest-blob` to wrap and sign) or a
//!   complete in-toto v1 Statement. Signing happens in CI via `cosign`;
//!   this layer owns the reproducible, testable provenance data.
//! - [`manifest`] — structured publish records (what each artifact shipped:
//!   tag, version, registry, digest), emitted per row and merged into a
//!   release manifest, so the workflow never scrapes a tool's stdout.
//!
//! # Example
//!
//! Compute the next version each group would move to, without writing
//! anything:
//!
//! ```no_run
//! use porter_core::{Config, ChangesetSet, current_versions, compute_next_version};
//! use std::path::Path;
//!
//! # fn main() -> anyhow::Result<()> {
//! let root = Path::new(".");
//! let config_path = Config::discover(root)
//!     .ok_or_else(|| anyhow::anyhow!("no porter.toml found"))?;
//! let config = Config::load(&config_path)?;
//! let set = ChangesetSet::load_from_dir(&root.join(&config.changesets.directory))?;
//! for (group, current) in current_versions(root, &config)? {
//!     if let Some(next) = compute_next_version(&current, &set.for_group(&group))? {
//!         println!("{group}: {} -> {}", next.previous, next.next);
//!     }
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
pub mod groups;
pub mod manifest;
pub mod matrix;
pub mod slug;
pub mod version;
pub mod versioned_files;

pub use apply::{
    ApplyResult, GroupApply, apply_next_version, current_versions, group_current_version,
    release_tags,
};
pub use attest::{AttestInput, Statement, build_provenance, build_statement, sha256_hex};
pub use build::{BuildArtifact, BuildOpts, append_checksum, build_cli_binary};
pub use changelog::{prepend_section, render_section, today_utc};
pub use changeset::{Bump, Changeset, ChangesetSet, write_changeset};
pub use config::{
    Artifact, AuthConfig, Component, Config, Group, Registry, RegistryKind, SigningBackend,
    VersionSource,
};
pub use groups::validate_changeset_groups;
pub use manifest::{PublishRecord, manifest, manifest_from_json};
pub use matrix::{MatrixRow, build_matrix, render_for_actions};
pub use slug::slugify;
pub use version::{NextVersion, compute_next_version};
pub use versioned_files::{VersionedFile, load_versioned_file};
