use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Bump {
    Patch,
    Minor,
    Major,
}

impl Bump {
    // `Ord::max(self, other)` is auto-derived from the variant order
    // (`Patch < Minor < Major`); aggregation uses it directly. We deliberately
    // do not define an inherent `max` to avoid colliding with the trait method.

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Patch => "patch",
            Self::Minor => "minor",
            Self::Major => "major",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changeset {
    pub path: PathBuf,
    pub bump: Bump,
    pub summary: String,
}

#[derive(Debug, Clone, Default)]
pub struct ChangesetSet {
    pub changesets: Vec<Changeset>,
}

impl ChangesetSet {
    /// Load every `*.md` file in `dir` (excluding `README.md`) as a
    /// changeset. Missing directory yields an empty set rather than an
    /// error so a brand-new repo with no `.changeset/` is well-formed.
    ///
    /// # Errors
    ///
    /// Returns an error if a file in the directory cannot be read, or
    /// any file's frontmatter fails to parse.
    pub fn load_from_dir(dir: &Path) -> Result<Self> {
        if !dir.exists() {
            return Ok(Self::default());
        }
        let mut changesets = Vec::new();
        for entry in
            fs::read_dir(dir).with_context(|| format!("reading changeset dir {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            if path.file_name().and_then(|s| s.to_str()) == Some("README.md") {
                continue;
            }
            let body = fs::read_to_string(&path)
                .with_context(|| format!("reading changeset {}", path.display()))?;
            let cs = parse_changeset(&path, &body)
                .with_context(|| format!("parsing changeset {}", path.display()))?;
            changesets.push(cs);
        }
        changesets.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Self { changesets })
    }

    pub fn aggregate_bump(&self) -> Option<Bump> {
        self.changesets.iter().map(|c| c.bump).reduce(Ord::max)
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.changesets.is_empty()
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.changesets.len()
    }
}

#[derive(Deserialize)]
struct Frontmatter {
    #[serde(rename = "release")]
    release: Option<String>,
    #[serde(rename = "bump")]
    bump: Option<String>,
}

fn parse_changeset(path: &Path, body: &str) -> Result<Changeset> {
    let body = body.trim_start_matches('\u{feff}');
    let rest = body.strip_prefix("---").context("missing leading ---")?;
    let end = rest.find("\n---").context("missing trailing ---")?;
    let frontmatter = &rest[..end];
    let summary = rest[end + 4..].trim().to_owned();

    let fm: Frontmatter =
        serde_yaml::from_str(frontmatter).context("failed to parse changeset frontmatter")?;
    let bump_str = fm
        .bump
        .or(fm.release)
        .context("changeset frontmatter must specify `bump:` or `release:`")?;
    let bump = match bump_str.trim().to_ascii_lowercase().as_str() {
        "patch" => Bump::Patch,
        "minor" => Bump::Minor,
        "major" | "breaking" => Bump::Major,
        other => bail!("unknown bump kind: {other:?}"),
    };

    Ok(Changeset {
        path: path.to_path_buf(),
        bump,
        summary,
    })
}

/// Write a new changeset Markdown file at `<dir>/<slug>.md`.
///
/// # Errors
///
/// Returns an error if `dir` cannot be created or the file cannot be
/// written.
pub fn write_changeset(dir: &Path, slug: &str, bump: Bump, summary: &str) -> Result<PathBuf> {
    fs::create_dir_all(dir).with_context(|| format!("creating changeset dir {}", dir.display()))?;
    let path = dir.join(format!("{slug}.md"));
    let body = format!(
        "---\nbump: {bump}\n---\n\n{summary}\n",
        bump = bump.as_str(),
        summary = summary.trim()
    );
    fs::write(&path, body).with_context(|| format!("writing changeset {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use tempfile::TempDir;

    #[test]
    fn parses_minor_changeset() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("foo.md");
        let body = indoc! {"
            ---
            bump: minor
            ---

            Add a new thing.
        "};
        std::fs::write(&path, body).unwrap();
        let cs = parse_changeset(&path, body).unwrap();
        assert_eq!(cs.bump, Bump::Minor);
        assert_eq!(cs.summary, "Add a new thing.");
    }

    #[test]
    fn aggregates_to_max_bump() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.md"), "---\nbump: patch\n---\n\nfix a\n").unwrap();
        std::fs::write(dir.path().join("b.md"), "---\nbump: minor\n---\n\nfeat b\n").unwrap();
        std::fs::write(dir.path().join("c.md"), "---\nbump: patch\n---\n\nfix c\n").unwrap();
        let set = ChangesetSet::load_from_dir(dir.path()).unwrap();
        assert_eq!(set.len(), 3);
        assert_eq!(set.aggregate_bump(), Some(Bump::Minor));
    }

    #[test]
    fn skips_readme() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("README.md"), "ignore me").unwrap();
        std::fs::write(
            dir.path().join("real.md"),
            "---\nbump: major\n---\n\nbreak\n",
        )
        .unwrap();
        let set = ChangesetSet::load_from_dir(dir.path()).unwrap();
        assert_eq!(set.len(), 1);
        assert_eq!(set.aggregate_bump(), Some(Bump::Major));
    }

    #[test]
    fn empty_dir_yields_empty_set() {
        let dir = TempDir::new().unwrap();
        let set = ChangesetSet::load_from_dir(dir.path()).unwrap();
        assert!(set.is_empty());
        assert_eq!(set.aggregate_bump(), None);
    }

    #[test]
    fn missing_dir_yields_empty_set() {
        let set = ChangesetSet::load_from_dir(Path::new("/does/not/exist/at/all")).unwrap();
        assert!(set.is_empty());
    }

    #[test]
    fn writes_and_roundtrips() {
        let dir = TempDir::new().unwrap();
        let p =
            write_changeset(dir.path(), "neat-feature", Bump::Minor, "Add neat feature.").unwrap();
        let set = ChangesetSet::load_from_dir(dir.path()).unwrap();
        assert_eq!(set.len(), 1);
        assert_eq!(set.changesets[0].bump, Bump::Minor);
        assert_eq!(set.changesets[0].summary, "Add neat feature.");
        assert_eq!(set.changesets[0].path, p);
    }
}
