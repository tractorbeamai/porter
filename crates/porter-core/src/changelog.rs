use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result};
use semver::Version;
use time::OffsetDateTime;
use time::macros::format_description;

use crate::changeset::{Bump, ChangesetSet};

/// Render a Markdown changelog section for a single release.
///
/// Format mirrors the Keep a Changelog convention loosely, grouped by bump
/// kind. Output is deterministic (no timestamps) when `today` is provided
/// fixed; otherwise the current UTC date is used.
///
/// Trust boundary: changeset summaries are written by PR authors and
/// pasted in here verbatim — they are also the `--notes-file` that `gh
/// release create` renders on the public Release page. Treat changesets
/// as untrusted Markdown and review them on PR like any other content
/// landing in the repo.
#[must_use]
pub fn render_section(version: &Version, today: &str, set: &ChangesetSet) -> String {
    let mut out = String::new();
    out.push_str(&format!("## {version} — {today}\n\n"));

    let mut majors = Vec::new();
    let mut minors = Vec::new();
    let mut patches = Vec::new();
    for c in &set.changesets {
        match c.bump {
            Bump::Major => majors.push(c),
            Bump::Minor => minors.push(c),
            Bump::Patch => patches.push(c),
        }
    }

    let mut group = |label: &str, entries: &[&crate::changeset::Changeset]| {
        if entries.is_empty() {
            return;
        }
        out.push_str(&format!("### {label}\n\n"));
        for c in entries {
            for (i, line) in c.summary.lines().enumerate() {
                if i == 0 {
                    out.push_str(&format!("- {line}\n"));
                } else {
                    out.push_str(&format!("  {line}\n"));
                }
            }
        }
        out.push('\n');
    };

    group("Breaking changes", &majors);
    group("Features", &minors);
    group("Fixes", &patches);

    out
}

/// Prepend a new section to the changelog file. Creates the file with a
/// standard header if it doesn't yet exist.
///
/// # Errors
///
/// Returns an error if the changelog file cannot be written, or its
/// parent directory cannot be created.
pub fn prepend_section(path: &Path, section: &str) -> Result<()> {
    let header = "# Changelog\n\n";
    let existing = fs::read_to_string(path).unwrap_or_default();
    let body = if existing.is_empty() {
        format!("{header}{section}")
    } else if let Some(rest) = existing.strip_prefix(header) {
        format!("{header}{section}{rest}")
    } else {
        // No recognized header — prepend ours and the new section ahead of
        // whatever was there.
        format!("{header}{section}{existing}")
    };
    if let Some(parent) = path.parent() {
        // Best-effort: if the parent already exists, this is a no-op;
        // any real failure surfaces on the subsequent `fs::write`.
        let _ = fs::create_dir_all(parent);
    }
    fs::write(path, body).with_context(|| format!("writing changelog {}", path.display()))?;
    Ok(())
}

/// Today's date in UTC, formatted as `YYYY-MM-DD`.
///
/// # Panics
///
/// In theory, if `time`'s formatter rejects the static literal
/// `[year]-[month]-[day]` against `OffsetDateTime::now_utc()`. Neither
/// can happen — the format string parses at compile time via
/// `format_description!`, and `now_utc()` is always in range — so
/// documenting the invariant beats smuggling an unreachable fallback
/// into release output.
#[must_use]
pub fn today_utc() -> String {
    let fmt = format_description!("[year]-[month]-[day]");
    #[expect(
        clippy::expect_used,
        reason = "documented in this fn's `# Panics`: literal format + always-valid time cannot fail"
    )]
    OffsetDateTime::now_utc()
        .format(&fmt)
        .expect("yyyy-mm-dd literal cannot fail to format")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::{Changeset, ChangesetSet};
    use indoc::indoc;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn cs(bump: Bump, summary: &str) -> Changeset {
        Changeset {
            path: PathBuf::from("x.md"),
            bump,
            summary: summary.into(),
        }
    }

    #[test]
    fn renders_grouped_section() {
        let set = ChangesetSet {
            changesets: vec![
                cs(Bump::Minor, "Add `attest` subcommand."),
                cs(Bump::Patch, "Fix flaky timestamp formatter."),
                cs(Bump::Major, "Rename `release tag` to `release cut`."),
            ],
        };
        let s = render_section(&Version::new(0, 6, 0), "2026-05-07", &set);
        let expected = indoc! {"
            ## 0.6.0 — 2026-05-07

            ### Breaking changes

            - Rename `release tag` to `release cut`.

            ### Features

            - Add `attest` subcommand.

            ### Fixes

            - Fix flaky timestamp formatter.

        "};
        pretty_assertions::assert_eq!(s, expected);
    }

    #[test]
    fn prepend_creates_file_when_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("CHANGELOG.md");
        let section = "## 0.1.0 — 2026-05-07\n\n### Features\n\n- Initial.\n\n";
        prepend_section(&path, section).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.starts_with("# Changelog\n\n## 0.1.0"));
    }

    #[test]
    fn prepend_handles_changelog_with_non_standard_header() {
        // If the existing changelog has a hand-rolled header we don't
        // recognize, we shouldn't lose its content — the new section is
        // inserted between our standard header and whatever was there.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("CHANGELOG.md");
        let custom = "# Project history\n\nFreeform notes about the project.\n";
        fs::write(&path, custom).unwrap();
        let section = "## 0.2.0 — 2026-05-07\n\n- New thing.\n\n";
        prepend_section(&path, section).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.starts_with("# Changelog\n\n## 0.2.0"));
        assert!(
            body.contains("# Project history"),
            "must not lose the original header; got:\n{body}"
        );
        assert!(body.contains("Freeform notes about the project."));
    }

    #[test]
    fn prepend_inserts_above_existing_sections() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("CHANGELOG.md");
        fs::write(
            &path,
            "# Changelog\n\n## 0.1.0 — 2026-05-01\n\n- Initial.\n\n",
        )
        .unwrap();
        let section = "## 0.2.0 — 2026-05-07\n\n- New thing.\n\n";
        prepend_section(&path, section).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("## 0.2.0"));
        assert!(body.contains("## 0.1.0"));
        let p2 = body.find("## 0.2.0").unwrap();
        let p1 = body.find("## 0.1.0").unwrap();
        assert!(p2 < p1, "0.2.0 must appear before 0.1.0");
    }
}
