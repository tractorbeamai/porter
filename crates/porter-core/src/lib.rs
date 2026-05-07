//! porter-core — release-cutting machinery for polyglot monorepos.
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
//!   the pre-1.0 convention.
//! - [`changelog`] — render Markdown sections and prepend them to a
//!   `CHANGELOG.md`.
//! - [`apply`] — orchestrate the whole "version" step: read current,
//!   compute next, rewrite every file, prepend changelog, consume
//!   changesets.

pub mod apply;
pub mod build;
pub mod changelog;
pub mod changeset;
pub mod config;
pub mod matrix;
pub mod slug;
pub mod version;
pub mod versioned_files;

pub use apply::{ApplyResult, apply_next_version, current_version};
pub use build::{BuildArtifact, BuildOpts, append_checksum, build_cli_binary};
pub use changelog::{prepend_section, render_section, today_utc};
pub use changeset::{Bump, Changeset, ChangesetSet, write_changeset};
pub use config::{ArtifactConfig, ChangesetMode, Config, SigningBackend, VersionedFileSpec};
pub use matrix::{MatrixRow, build_matrix, render_for_actions};
pub use slug::slugify;
pub use version::{NextVersion, compute_next_version};
pub use versioned_files::{VersionedFile, load_versioned_file};
