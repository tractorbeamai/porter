use anyhow::Result;
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::changeset::{Bump, ChangesetSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NextVersion {
    pub previous: Version,
    pub next: Version,
    pub bump: Bump,
}

/// Compute the next version given the current version and a set of changesets.
///
/// Returns `None` if the changeset set is empty (no bump).
///
/// # Errors
///
/// Currently infallible; the `Result` is reserved for future bump
/// strategies (e.g. independent-mode aggregation that can detect
/// conflicting bumps).
pub fn compute_next_version(current: &Version, set: &ChangesetSet) -> Result<Option<NextVersion>> {
    let Some(bump) = set.aggregate_bump() else {
        return Ok(None);
    };
    let next = bumped(current, bump);
    Ok(Some(NextVersion {
        previous: current.clone(),
        next,
        bump,
    }))
}

const fn bumped(v: &Version, bump: Bump) -> Version {
    // Under 1.0.0, SemVer's "initial development" clause treats the public
    // API as unstable, so digit significance shifts left: a `major` change
    // bumps the minor (0.1.0 -> 0.2.0) while `minor` and `patch` both bump
    // the patch (0.1.0 -> 0.1.1). This is why a minor changeset produces a
    // patch-position bump pre-1.0. At/after 1.0.0 the usual rules apply.
    // Matches Changesets and Cargo's 0.x compatibility handling.
    // https://semver.org/#spec-item-4
    match bump {
        Bump::Major => {
            if v.major == 0 {
                Version::new(0, v.minor + 1, 0)
            } else {
                Version::new(v.major + 1, 0, 0)
            }
        }
        Bump::Minor => {
            if v.major == 0 {
                Version::new(0, v.minor, v.patch + 1)
            } else {
                Version::new(v.major, v.minor + 1, 0)
            }
        }
        Bump::Patch => Version::new(v.major, v.minor, v.patch + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::Changeset;
    use std::path::PathBuf;

    fn cs(bump: Bump) -> Changeset {
        Changeset {
            path: PathBuf::from("a.md"),
            bump,
            summary: String::new(),
        }
    }

    #[test]
    fn empty_set_returns_none() {
        let set = ChangesetSet::default();
        let nv = compute_next_version(&Version::new(0, 1, 0), &set).unwrap();
        assert!(nv.is_none());
    }

    #[test]
    fn patch_bump() {
        let set = ChangesetSet {
            changesets: vec![cs(Bump::Patch)],
        };
        let nv = compute_next_version(&Version::new(1, 2, 3), &set)
            .unwrap()
            .unwrap();
        assert_eq!(nv.next, Version::new(1, 2, 4));
    }

    #[test]
    fn minor_bump_post_1() {
        let set = ChangesetSet {
            changesets: vec![cs(Bump::Minor)],
        };
        let nv = compute_next_version(&Version::new(1, 2, 3), &set)
            .unwrap()
            .unwrap();
        assert_eq!(nv.next, Version::new(1, 3, 0));
    }

    #[test]
    fn minor_bump_pre_1_is_patch() {
        let set = ChangesetSet {
            changesets: vec![cs(Bump::Minor)],
        };
        let nv = compute_next_version(&Version::new(0, 5, 2), &set)
            .unwrap()
            .unwrap();
        assert_eq!(nv.next, Version::new(0, 5, 3));
    }

    #[test]
    fn major_bump_post_1() {
        let set = ChangesetSet {
            changesets: vec![cs(Bump::Major)],
        };
        let nv = compute_next_version(&Version::new(2, 4, 9), &set)
            .unwrap()
            .unwrap();
        assert_eq!(nv.next, Version::new(3, 0, 0));
    }

    #[test]
    fn major_bump_pre_1_is_minor() {
        let set = ChangesetSet {
            changesets: vec![cs(Bump::Major)],
        };
        let nv = compute_next_version(&Version::new(0, 5, 2), &set)
            .unwrap()
            .unwrap();
        assert_eq!(nv.next, Version::new(0, 6, 0));
    }

    #[test]
    fn aggregates_to_max_bump() {
        let set = ChangesetSet {
            changesets: vec![cs(Bump::Patch), cs(Bump::Minor), cs(Bump::Patch)],
        };
        let nv = compute_next_version(&Version::new(1, 2, 3), &set)
            .unwrap()
            .unwrap();
        assert_eq!(nv.bump, Bump::Minor);
        assert_eq!(nv.next, Version::new(1, 3, 0));
    }
}
