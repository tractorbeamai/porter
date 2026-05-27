//! Domain helpers over the group/component model.
//!
//! So the rest of the crate never re-walks the raw config structs: group
//! lookup, the version sources and artifacts a group owns, a component's tag,
//! and changeset→group reference checking.

use anyhow::{Result, bail};
use semver::Version;

use crate::changeset::ChangesetSet;
use crate::config::{Artifact, Component, Config, Group, ReleaseConfig, VersionSource};

impl Group {
    /// The version sources this group owns (components that carry one). These
    /// must agree on a current version — that's the group's release line.
    pub fn version_sources(&self) -> impl Iterator<Item = &VersionSource> {
        self.components.iter().filter_map(|c| c.version.as_ref())
    }

    /// The components in this group that publish an artifact.
    pub fn artifact_components(&self) -> impl Iterator<Item = &Component> {
        self.components.iter().filter(|c| c.artifact.is_some())
    }

    /// The changelog this group writes to: its own override, else the
    /// repo-wide default.
    #[must_use]
    pub fn changelog_path<'a>(&'a self, release: &'a ReleaseConfig) -> &'a std::path::Path {
        self.changelog.as_deref().unwrap_or(&release.changelog)
    }
}

impl Component {
    /// The tag stem for this component: its `tag_prefix` override, else
    /// `<id>/v` (changesets-style per-package tags).
    #[must_use]
    pub fn tag_stem(&self) -> String {
        self.tag_prefix
            .clone()
            .unwrap_or_else(|| format!("{}/v", self.id))
    }

    /// This component's tag at `version`, e.g. `py-sdk/v0.4.1` or `v0.1.0`.
    #[must_use]
    pub fn tag(&self, version: &Version) -> String {
        format!("{}{version}", self.tag_stem())
    }

    /// The `artifact` paired with this component's id, for callers that build
    /// or publish it.
    #[must_use]
    pub const fn artifact(&self) -> Option<&Artifact> {
        self.artifact.as_ref()
    }
}

/// Verify every group a changeset names actually exists.
///
/// A changeset that omits `groups` is allowed only when there's exactly one
/// group (it defaults to that group); otherwise the author must say which
/// line(s) it bumps.
///
/// # Errors
///
/// Returns an error naming the changeset and the unknown or ambiguous group.
pub fn validate_changeset_groups(config: &Config, set: &ChangesetSet) -> Result<()> {
    let only_group = (config.groups.len() == 1).then(|| config.groups[0].name.as_str());
    for cs in &set.changesets {
        if cs.groups.is_empty() && only_group.is_none() {
            bail!(
                "changeset {} names no `groups:` but the repo has {} groups; \
                 list the group(s) it bumps",
                cs.path.display(),
                config.groups.len()
            );
        }
        for g in &cs.groups {
            if config.group(g).is_none() {
                bail!(
                    "changeset {} targets unknown group {:?}",
                    cs.path.display(),
                    g
                );
            }
        }
    }
    Ok(())
}
