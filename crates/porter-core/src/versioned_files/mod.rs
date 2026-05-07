use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use semver::Version;

use crate::config::VersionedFileSpec;

mod cargo_workspace;
mod helm_chart;
mod package_json;
mod regex_file;

pub use cargo_workspace::CargoWorkspaceFile;
pub use helm_chart::HelmChartFile;
pub use package_json::PackageJsonFile;
pub use regex_file::RegexFile;

/// A file in the repo whose embedded version string moves in lockstep
/// with the release version.
pub trait VersionedFile {
    /// Path of the file (relative to the config root).
    fn path(&self) -> &Path;

    /// Read the current version from the file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or its contents
    /// don't match the expected format for the adapter.
    fn read_version(&self) -> Result<Version>;

    /// Rewrite the file with the given version. Must preserve formatting,
    /// comments, and unrelated keys wherever possible.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or written.
    fn write_version(&self, version: &Version) -> Result<()>;
}

/// Construct an adapter for a single `[[versioned_files]]` entry.
///
/// `root` is the directory the spec's `path` is resolved against (typically
/// the directory containing `porter.toml`).
///
/// # Errors
///
/// Returns an error if the spec's regex pattern fails to compile or is
/// missing the required `version` named group.
pub fn load_versioned_file(
    root: &Path,
    spec: &VersionedFileSpec,
) -> Result<Box<dyn VersionedFile>> {
    let resolve = |p: &Path| -> PathBuf {
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            root.join(p)
        }
    };
    Ok(match spec {
        VersionedFileSpec::CargoWorkspace { path } => {
            Box::new(CargoWorkspaceFile::new(resolve(path)))
        }
        VersionedFileSpec::HelmChart {
            path,
            update_app_version,
        } => Box::new(HelmChartFile::new(resolve(path), *update_app_version)),
        VersionedFileSpec::PackageJson { path } => Box::new(PackageJsonFile::new(resolve(path))),
        VersionedFileSpec::Regex { path, pattern } => Box::new(
            RegexFile::new(resolve(path), pattern)
                .context("compiling regex versioned-file pattern")?,
        ),
    })
}
