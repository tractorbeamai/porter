use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use semver::Version;

use super::VersionedFile;

/// `package.json` adapter.
///
/// Updates the top-level `"version"` field via a structural JSON walk that
/// rewrites only the matched span. We deliberately avoid
/// `serde_json::Value` round-trips because they reorder keys and discard
/// formatting that npm tooling and humans both rely on; the same approach
/// `pnpm version` and `npm version` take.
#[derive(Debug)]
pub struct PackageJsonFile {
    path: PathBuf,
}

impl PackageJsonFile {
    #[must_use]
    pub const fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl VersionedFile for PackageJsonFile {
    fn path(&self) -> &Path {
        &self.path
    }

    fn read_version(&self) -> Result<Version> {
        let body = fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        // npm tooling tolerates a UTF-8 BOM at the head of package.json; we
        // shouldn't be stricter than the ecosystem.
        let json = body.strip_prefix('\u{feff}').unwrap_or(&body);
        let v: serde_json::Value = serde_json::from_str(json)
            .with_context(|| format!("invalid JSON in {}", self.path.display()))?;
        let v = v.get("version").and_then(|v| v.as_str()).with_context(|| {
            format!(
                "{} has no top-level \"version\" string",
                self.path.display()
            )
        })?;
        Version::parse(v).with_context(|| format!("parsing version {v:?}"))
    }

    fn write_version(&self, version: &Version) -> Result<()> {
        let body = fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        let updated = replace_top_level_version(&body, &version.to_string())
            .with_context(|| format!("rewriting version in {}", self.path.display()))?;
        fs::write(&self.path, updated)
            .with_context(|| format!("writing {}", self.path.display()))?;
        Ok(())
    }
}

fn replace_top_level_version(body: &str, new_value: &str) -> Result<String> {
    let span = find_top_level_string_span(body, "version")
        .context("no top-level \"version\" field found")?;
    let mut out = String::with_capacity(body.len() + new_value.len());
    out.push_str(&body[..span.start]);
    out.push_str(new_value);
    out.push_str(&body[span.end..]);
    Ok(out)
}

#[derive(Debug, PartialEq, Eq)]
struct Span {
    start: usize,
    end: usize,
}

/// Scan a JSON document for the byte range of the *string value* assigned
/// to the named top-level key. Walks the document structurally so nested
/// occurrences of the same key inside arrays or sub-objects are skipped.
fn find_top_level_string_span(body: &str, key: &str) -> Option<Span> {
    let bytes = body.as_bytes();
    // Skip a UTF-8 BOM if present; the rebuild splices unmodified bytes
    // around the rewritten span, so the BOM is preserved verbatim.
    let bom_len = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else {
        0
    };
    let mut i = skip_ws(bytes, bom_len);
    if i >= bytes.len() || bytes[i] != b'{' {
        return None;
    }
    i += 1;
    loop {
        i = skip_ws(bytes, i);
        if i >= bytes.len() {
            return None;
        }
        if bytes[i] == b'}' {
            return None;
        }
        if bytes[i] == b',' {
            i += 1;
            continue;
        }
        // Expect a string key.
        let key_span = read_string(bytes, i)?;
        let this_key = &body[key_span.start + 1..key_span.end - 1];
        i = key_span.end;
        i = skip_ws(bytes, i);
        if i >= bytes.len() || bytes[i] != b':' {
            return None;
        }
        i += 1;
        i = skip_ws(bytes, i);
        if i >= bytes.len() {
            return None;
        }
        if this_key == key {
            // Must be a string value; if not, bail rather than silently
            // pick up a non-string `version: 1` (rare but possible).
            if bytes[i] != b'"' {
                return None;
            }
            let val = read_string(bytes, i)?;
            return Some(Span {
                start: val.start + 1,
                end: val.end - 1,
            });
        }
        // Skip the value of a different key.
        i = skip_value(bytes, i)?;
    }
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

fn read_string(bytes: &[u8], start: usize) -> Option<Span> {
    if start >= bytes.len() || bytes[start] != b'"' {
        return None;
    }
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                i += 2;
            }
            b'"' => {
                return Some(Span { start, end: i + 1 });
            }
            _ => i += 1,
        }
    }
    None
}

fn skip_value(bytes: &[u8], start: usize) -> Option<usize> {
    let i = skip_ws(bytes, start);
    if i >= bytes.len() {
        return None;
    }
    match bytes[i] {
        b'"' => Some(read_string(bytes, i)?.end),
        b'{' | b'[' => Some(skip_balanced(bytes, i)?),
        _ => {
            // number / true / false / null
            let mut j = i;
            while j < bytes.len()
                && !matches!(bytes[j], b',' | b'}' | b']' | b' ' | b'\n' | b'\r' | b'\t')
            {
                j += 1;
            }
            Some(j)
        }
    }
}

fn skip_balanced(bytes: &[u8], start: usize) -> Option<usize> {
    let open = bytes[start];
    let close = match open {
        b'{' => b'}',
        b'[' => b']',
        _ => return None,
    };
    let mut depth = 0_i32;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let s = read_string(bytes, i)?;
                i = s.end;
                continue;
            }
            c if c == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use tempfile::TempDir;

    fn setup(body: &str) -> (TempDir, PackageJsonFile) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("package.json");
        fs::write(&path, body).unwrap();
        let f = PackageJsonFile::new(path);
        (dir, f)
    }

    #[test]
    fn reads_version() {
        let (_d, f) = setup(indoc! {r#"
            {
              "name": "example",
              "version": "0.1.0",
              "private": true
            }
        "#});
        assert_eq!(f.read_version().unwrap(), Version::new(0, 1, 0));
    }

    #[test]
    fn writes_preserves_formatting_and_other_fields() {
        let body = indoc! {r#"
            {
              "name": "example",
              "version": "0.1.0",
              "private": true,
              "scripts": {
                "build": "tsc"
              },
              "dependencies": {
                "left-pad": "1.3.0"
              }
            }
        "#};
        let (_d, f) = setup(body);
        f.write_version(&Version::new(0, 6, 0)).unwrap();
        let after = fs::read_to_string(f.path()).unwrap();
        assert!(after.contains(r#""version": "0.6.0","#));
        // formatting preserved
        assert!(after.contains("  \"scripts\": {"));
        assert!(after.contains("\"build\": \"tsc\""));
        // dependency version untouched
        assert!(after.contains(r#""left-pad": "1.3.0""#));
    }

    #[test]
    fn does_not_match_dependency_version() {
        let body = indoc! {r#"
            {
              "name": "example",
              "dependencies": {
                "version": "1.2.3"
              },
              "version": "0.1.0"
            }
        "#};
        let (_d, f) = setup(body);
        f.write_version(&Version::new(0, 2, 0)).unwrap();
        let after = fs::read_to_string(f.path()).unwrap();
        assert!(after.contains(r#""version": "0.2.0""#));
        assert!(after.contains(r#""version": "1.2.3""#));
    }

    #[test]
    fn reads_with_escaped_quote_in_earlier_string() {
        let (_d, f) = setup(indoc! {r#"
            {
              "name": "example",
              "description": "say \"hi\"",
              "version": "0.1.0"
            }
        "#});
        assert_eq!(f.read_version().unwrap(), Version::new(0, 1, 0));
    }

    #[test]
    fn writes_when_version_is_last_field_no_trailing_comma() {
        let body = indoc! {r#"
            {
              "name": "example",
              "version": "0.1.0"
            }
        "#};
        let (_d, f) = setup(body);
        f.write_version(&Version::new(0, 2, 0)).unwrap();
        let after = fs::read_to_string(f.path()).unwrap();
        assert!(after.contains(r#""version": "0.2.0""#));
        let parsed: serde_json::Value = serde_json::from_str(&after).unwrap();
        assert_eq!(parsed["version"], "0.2.0");
    }

    #[test]
    fn roundtrips_crlf_line_endings() {
        let body = "{\r\n  \"name\": \"example\",\r\n  \"version\": \"0.1.0\"\r\n}\r\n";
        let (_d, f) = setup(body);
        f.write_version(&Version::new(0, 2, 0)).unwrap();
        let after = fs::read_to_string(f.path()).unwrap();
        assert_eq!(
            after,
            "{\r\n  \"name\": \"example\",\r\n  \"version\": \"0.2.0\"\r\n}\r\n"
        );
        assert_eq!(f.read_version().unwrap(), Version::new(0, 2, 0));
    }

    #[test]
    fn reads_with_utf8_bom() {
        let body = format!(
            "\u{feff}{}",
            indoc! {r#"
                {
                  "name": "example",
                  "version": "0.1.0"
                }
            "#}
        );
        let (_d, f) = setup(&body);
        assert_eq!(f.read_version().unwrap(), Version::new(0, 1, 0));
    }

    #[test]
    fn writes_with_utf8_bom_preserves_bom() {
        let body = format!(
            "\u{feff}{}",
            indoc! {r#"
                {
                  "name": "example",
                  "version": "0.1.0"
                }
            "#}
        );
        let (_d, f) = setup(&body);
        f.write_version(&Version::new(0, 2, 0)).unwrap();
        let after = fs::read_to_string(f.path()).unwrap();
        assert!(after.starts_with('\u{feff}'));
        assert!(after.contains(r#""version": "0.2.0""#));
    }

    #[test]
    fn missing_version_errors() {
        let (_d, f) = setup(indoc! {r#"
            {
              "name": "example"
            }
        "#});
        let err = f.read_version().unwrap_err().to_string();
        assert!(err.contains("version"));
    }
}
