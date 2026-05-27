use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow, bail};
use semver::Version;

use crate::changelog::{prepend_section, render_section, today_utc};
use crate::changeset::ChangesetSet;
use crate::config::{Config, Group};
use crate::groups::validate_changeset_groups;
use crate::version::{NextVersion, compute_next_version};
use crate::versioned_files::load_versioned_file;

/// What one group's release did: the version it moved to, the files rewritten,
/// the changelog it wrote, and the tags its published components produce.
#[derive(Debug, Clone)]
pub struct GroupApply {
    pub group: String,
    pub next: NextVersion,
    pub rewritten_files: Vec<PathBuf>,
    pub changelog_path: PathBuf,
    pub tags: Vec<String>,
}

/// Result of running `porter version`: the per-group bumps plus the changeset
/// files consumed across all of them.
#[derive(Debug, Clone)]
pub struct ApplyResult {
    pub groups: Vec<GroupApply>,
    pub consumed_changesets: Vec<PathBuf>,
}

/// Read a single group's current version from its version sources, asserting
/// they agree.
///
/// Disagreement *within* a group is the drift porter exists to prevent;
/// disagreement *across* groups is expected and fine — that's the whole point
/// of groups.
///
/// # Errors
///
/// Returns an error if the group has no version source, any source can't be
/// read, or its sources disagree.
pub fn group_current_version(root: &Path, group: &Group) -> Result<Version> {
    let mut versions: Vec<(PathBuf, Version)> = Vec::new();
    for spec in group.version_sources() {
        let f = load_versioned_file(root, spec)?;
        let v = f
            .read_version()
            .with_context(|| format!("reading current version from {}", f.path().display()))?;
        versions.push((f.path().to_path_buf(), v));
    }
    let (first_path, first) = versions
        .first()
        .ok_or_else(|| anyhow!("group {:?} has no version source", group.name))?;
    for (p, v) in &versions[1..] {
        if v != first {
            bail!(
                "group {:?}: versioned files disagree on current version: \
                 {} reports {}, {} reports {}",
                group.name,
                first_path.display(),
                first,
                p.display(),
                v
            );
        }
    }
    Ok(first.clone())
}

/// Each group's current version, keyed by group name.
///
/// # Errors
///
/// Returns an error if any group's [`group_current_version`] fails.
pub fn current_versions(root: &Path, config: &Config) -> Result<BTreeMap<String, Version>> {
    config
        .groups
        .iter()
        .map(|g| Ok((g.name.clone(), group_current_version(root, g)?)))
        .collect()
}

/// Every tag the current tree implies.
///
/// For each group, the tag of each published (artifact-bearing) component at
/// that group's current version. `porter release tag` prints these; the
/// workflow pushes the ones that don't already exist, so unchanged groups are
/// naturally skipped.
///
/// # Errors
///
/// Returns an error if any group's current version can't be read.
pub fn release_tags(root: &Path, config: &Config) -> Result<Vec<String>> {
    let mut tags = Vec::new();
    for group in &config.groups {
        let version = group_current_version(root, group)?;
        for component in group.artifact_components() {
            tags.push(component.tag(&version));
        }
    }
    Ok(tags)
}

/// Compute and apply each group's next version.
///
/// For every group with pending changesets: read its current version, compute
/// the next, rewrite its version sources, and prepend a rendered section to
/// its changelog. Consumed changesets are removed at the end. With `dry_run`,
/// nothing is written.
///
/// # Errors
///
/// Returns an error if changesets are malformed, reference unknown groups, a
/// group's current version can't be read, or any write fails.
pub fn apply_next_version(
    root: &Path,
    config: &Config,
    dry_run: bool,
) -> Result<Option<ApplyResult>> {
    let set = ChangesetSet::load_from_dir(&root.join(&config.changesets.directory))?;
    if set.is_empty() {
        return Ok(None);
    }
    validate_changeset_groups(config, &set)?;

    let today = today_utc();
    let mut applies = Vec::new();
    for group in &config.groups {
        let group_set = set.for_group(&group.name);
        if group_set.is_empty() {
            continue;
        }
        let current = group_current_version(root, group)?;
        let Some(next) = compute_next_version(&current, &group_set)? else {
            continue;
        };

        let mut rewritten = Vec::new();
        for spec in group.version_sources() {
            let f = load_versioned_file(root, spec)?;
            if !dry_run {
                f.write_version(&next.next)
                    .with_context(|| format!("writing version to {}", f.path().display()))?;
            }
            rewritten.push(f.path().to_path_buf());
        }

        let changelog_path = root.join(group.changelog_path(&config.release));
        if !dry_run {
            let section = render_section(&next.next, &today, &group_set);
            prepend_section(&changelog_path, &section)?;
        }

        let tags = group
            .artifact_components()
            .map(|c| c.tag(&next.next))
            .collect();
        applies.push(GroupApply {
            group: group.name.clone(),
            next,
            rewritten_files: rewritten,
            changelog_path,
            tags,
        });
    }

    if applies.is_empty() {
        return Ok(None);
    }

    let consumed = set
        .changesets
        .iter()
        .map(|c| c.path.clone())
        .collect::<Vec<_>>();
    if !dry_run {
        // Removal is not transactional: a mid-loop failure leaves some files
        // bumped, some changelogs rewritten, and a partial set of changesets
        // gone. In practice this runs in CI on a fresh checkout and the
        // rolling Version PR is regenerated on the next push, so the partial
        // state never reaches main.
        for p in &consumed {
            fs::remove_file(p).with_context(|| format!("removing changeset {}", p.display()))?;
        }
    }

    Ok(Some(ApplyResult {
        groups: applies,
        consumed_changesets: consumed,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::{Bump, write_changeset};
    use indoc::indoc;
    use tempfile::TempDir;

    /// A single-group repo (porter-shaped): one `default` group whose four
    /// version sources move together.
    fn single_group_fixture() -> TempDir {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("porter.toml"),
            indoc! {r#"
                [[group]]
                name = "default"
                components = [
                  { id = "ws", type = "cargo-workspace", path = "Cargo.toml" },
                  { id = "chart", type = "helm-chart", path = "Chart.yaml" },
                  { id = "pkg", type = "package-json", path = "package.json" },
                  { id = "tf", type = "regex", path = "main.tf",
                    pattern = 'platform_chart_revision\s*=\s*"(?P<version>v[0-9.]+)"' },
                ]
            "#},
        )
        .unwrap();
        write_versioned_files(dir.path(), "0.1.0");
        fs::create_dir_all(dir.path().join(".changeset")).unwrap();
        dir
    }

    fn write_versioned_files(root: &Path, v: &str) {
        fs::write(
            root.join("Cargo.toml"),
            format!("[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"{v}\"\n"),
        )
        .unwrap();
        fs::write(
            root.join("Chart.yaml"),
            format!("apiVersion: v2\nname: example\nversion: {v}\nappVersion: \"{v}\"\n"),
        )
        .unwrap();
        fs::write(
            root.join("package.json"),
            format!("{{\n  \"name\": \"example\",\n  \"version\": \"{v}\"\n}}\n"),
        )
        .unwrap();
        fs::write(
            root.join("main.tf"),
            format!("platform_chart_revision = \"v{v}\"\n"),
        )
        .unwrap();
    }

    fn config(dir: &Path) -> Config {
        let body = fs::read_to_string(dir.join("porter.toml")).unwrap();
        Config::from_toml(&body).unwrap()
    }

    #[test]
    fn no_changesets_yields_none() {
        let dir = single_group_fixture();
        let r = apply_next_version(dir.path(), &config(dir.path()), false).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn applies_bump_across_all_files_in_group() {
        let dir = single_group_fixture();
        write_changeset(
            &dir.path().join(".changeset"),
            "feat-attest",
            Bump::Minor,
            &[],
            "Add attest subcommand.",
        )
        .unwrap();
        let r = apply_next_version(dir.path(), &config(dir.path()), false)
            .unwrap()
            .unwrap();
        assert_eq!(r.groups.len(), 1);
        // 0.1.0 + minor, pre-1.0 → patch position → 0.1.1
        assert_eq!(r.groups[0].next.next.to_string(), "0.1.1");
        for f in ["Cargo.toml", "Chart.yaml", "package.json"] {
            assert!(
                fs::read_to_string(dir.path().join(f))
                    .unwrap()
                    .contains("0.1.1"),
                "{f} not bumped"
            );
        }
        assert!(
            fs::read_to_string(dir.path().join("main.tf"))
                .unwrap()
                .contains("v0.1.1")
        );
        let cl = fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap();
        assert!(cl.contains("0.1.1") && cl.contains("Add attest subcommand."));
        assert_eq!(
            fs::read_dir(dir.path().join(".changeset")).unwrap().count(),
            0
        );
    }

    #[test]
    fn dry_run_does_not_mutate() {
        let dir = single_group_fixture();
        write_changeset(
            &dir.path().join(".changeset"),
            "feat",
            Bump::Major,
            &[],
            "breaking",
        )
        .unwrap();
        let _ = apply_next_version(dir.path(), &config(dir.path()), true)
            .unwrap()
            .unwrap();
        assert!(
            fs::read_to_string(dir.path().join("Cargo.toml"))
                .unwrap()
                .contains("0.1.0")
        );
        assert!(!dir.path().join("CHANGELOG.md").exists());
        assert_eq!(
            fs::read_dir(dir.path().join(".changeset")).unwrap().count(),
            1
        );
    }

    #[test]
    fn within_group_drift_errors() {
        let dir = single_group_fixture();
        // Bump only Cargo.toml so the group's sources disagree.
        fs::write(
            dir.path().join("Cargo.toml"),
            "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"0.2.0\"\n",
        )
        .unwrap();
        let err = current_versions(dir.path(), &config(dir.path()))
            .unwrap_err()
            .to_string();
        assert!(err.contains("disagree"), "{err}");
    }

    /// Two independent groups at different versions: a multi-group repo
    /// porter previously couldn't represent at all.
    fn multi_group_fixture() -> TempDir {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("porter.toml"),
            indoc! {r#"
                [[group]]
                name = "app"
                changelog = "APP_CHANGELOG.md"
                components = [
                  { id = "ws", type = "cargo-workspace", path = "Cargo.toml",
                    artifact = { kind = "cli-binary", package = "app-cli" } },
                ]

                [[group]]
                name = "charts"
                changelog = "CHARTS_CHANGELOG.md"
                components = [
                  { id = "chart", type = "helm-chart", path = "Chart.yaml",
                    artifact = { kind = "helm-chart", chart = ".", registry = "r" } },
                ]
            "#},
        )
        .unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("Chart.yaml"),
            "apiVersion: v2\nname: example\nversion: 1.4.2\nappVersion: \"1.4.2\"\n",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join(".changeset")).unwrap();
        dir
    }

    #[test]
    fn groups_bump_independently_with_distinct_versions() {
        let dir = multi_group_fixture();
        // Reading current versions does NOT error despite app=0.1.0, charts=1.4.2.
        let versions = current_versions(dir.path(), &config(dir.path())).unwrap();
        assert_eq!(versions["app"].to_string(), "0.1.0");
        assert_eq!(versions["charts"].to_string(), "1.4.2");

        write_changeset(
            &dir.path().join(".changeset"),
            "chart-fix",
            Bump::Patch,
            &["charts".to_owned()],
            "Fix the chart.",
        )
        .unwrap();
        let r = apply_next_version(dir.path(), &config(dir.path()), false)
            .unwrap()
            .unwrap();
        // Only charts bumped; app untouched.
        assert_eq!(r.groups.len(), 1);
        assert_eq!(r.groups[0].group, "charts");
        assert_eq!(r.groups[0].next.next.to_string(), "1.4.3");
        assert_eq!(r.groups[0].tags, vec!["chart/v1.4.3"]);
        assert!(
            fs::read_to_string(dir.path().join("Cargo.toml"))
                .unwrap()
                .contains("0.1.0")
        );
        assert!(
            fs::read_to_string(dir.path().join("Chart.yaml"))
                .unwrap()
                .contains("1.4.3")
        );
        // Per-group changelog routing.
        assert!(dir.path().join("CHARTS_CHANGELOG.md").exists());
        assert!(!dir.path().join("APP_CHANGELOG.md").exists());
    }

    #[test]
    fn release_tags_cover_published_components_per_group() {
        let dir = multi_group_fixture();
        let tags = release_tags(dir.path(), &config(dir.path())).unwrap();
        assert_eq!(tags, vec!["ws/v0.1.0", "chart/v1.4.2"]);
    }
}
