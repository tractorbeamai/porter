/// Turn a free-text changeset summary into a filesystem-safe slug.
///
/// Conservative: lowercases ASCII, replaces runs of non-alphanumeric
/// characters with a single dash, trims leading/trailing dashes, and caps
/// length. Non-ASCII characters are dropped (changeset filenames are not
/// user-facing).
pub fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_dash = true;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.len() > 60 {
        out.truncate(60);
        while out.ends_with('-') {
            out.pop();
        }
    }
    if out.is_empty() {
        "changeset".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic() {
        assert_eq!(slugify("Add new feature"), "add-new-feature");
    }

    #[test]
    fn collapses_runs() {
        assert_eq!(slugify("foo!!!  bar///baz"), "foo-bar-baz");
    }

    #[test]
    fn trims_dashes() {
        assert_eq!(slugify("---hi---"), "hi");
    }

    #[test]
    fn caps_length() {
        let long = "a".repeat(200);
        assert!(slugify(&long).len() <= 60);
    }

    #[test]
    fn drops_non_ascii() {
        assert_eq!(slugify("café résumé"), "caf-r-sum");
    }

    #[test]
    fn empty_falls_back() {
        assert_eq!(slugify("!!! ☃ ☃"), "changeset");
    }
}
