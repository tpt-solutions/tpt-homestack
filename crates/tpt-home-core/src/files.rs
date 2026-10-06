// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Safe filesystem helpers for the `data/uploads/` tree.

use std::path::{Component, Path, PathBuf};

use crate::error::ApiError;

/// Sanitize a client-supplied filename: keep the final component, strip
/// anything that is not a letter, digit, `.`, `_`, `-` or space, and clamp
/// the length. Never trust this to be unique — pair it with a random prefix.
pub fn sanitize_filename(name: &str) -> String {
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("file")
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '))
        .take(120)
        .collect::<String>();
    let trimmed = base.trim_matches('.').trim();
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Join `segments` onto `root`, rejecting absolute paths and traversal
/// (`..`, raw path components). The result is guaranteed to stay under
/// `root` lexically; callers dealing with pre-existing files should
/// re-check with [`path_within_canonical`].
pub fn safe_join(root: &Path, segments: &[&str]) -> Result<PathBuf, ApiError> {
    if segments.is_empty() {
        return Err(ApiError::bad_request("empty path"));
    }
    let mut path = root.to_path_buf();
    for seg in segments {
        let p = Path::new(seg);
        if p.is_absolute() {
            return Err(ApiError::bad_request("absolute paths are not allowed"));
        }
        for comp in p.components() {
            match comp {
                Component::Normal(part) => {
                    let part = part.to_string_lossy();
                    if part.contains(['\0', '\n', '\r']) {
                        return Err(ApiError::bad_request("invalid path characters"));
                    }
                    path.push(part.as_ref());
                }
                Component::CurDir => {}
                _ => return Err(ApiError::bad_request("path traversal rejected")),
            }
        }
    }
    debug_assert!(path.starts_with(root));
    Ok(path)
}

/// Canonical containment check for paths that may exist on disk.
pub fn path_within_canonical(root: &Path, path: &Path) -> bool {
    match (dunce_canonical(root), dunce_canonical(path)) {
        (Ok(r), Ok(p)) => p.starts_with(r),
        // Non-existent leaf: check its parent against the canonical root.
        (Ok(r), Err(_)) => path
            .parent()
            .map(dunce_canonical)
            .map(|p| p.is_ok_and(|p| p.starts_with(r)))
            .unwrap_or(false),
        _ => false,
    }
}

// std Canonicalize returns UNC paths on Windows which surprises callers;
// use `fs::canonicalize` but strip the `\\?\` prefix politely.
fn dunce_canonical(p: &Path) -> std::io::Result<PathBuf> {
    let c = std::fs::canonicalize(p)?;
    let s = c.as_os_str().to_string_lossy();
    let stripped = s.strip_prefix(r"\\?\").map(PathBuf::from).unwrap_or(c);
    Ok(stripped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_hostile_filenames() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("a/b\\c.txt"), "c.txt");
        assert_eq!(sanitize_filename("..\\..\\.."), "file");
        assert_eq!(sanitize_filename("\0\u{7f}.sh"), "sh");
        assert_eq!(sanitize_filename("my report (v2).md"), "my report v2.md");
    }

    #[test]
    fn safe_join_rejects_traversal() {
        let root = Path::new("/data/uploads");
        assert!(safe_join(root, &["drops", "x.bin"]).is_ok());
        assert!(safe_join(root, &["..", "etc"]).is_err());
        assert!(safe_join(root, &["/etc/passwd"]).is_err());
        assert!(safe_join(root, &["a/../../b"]).is_err());
        assert!(safe_join(root, &[]).is_err());
    }

    #[test]
    fn canonical_containment() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        let inside = root.join("sub");
        std::fs::create_dir_all(&inside).unwrap();
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();

        assert!(path_within_canonical(&root, &inside));
        assert!(!path_within_canonical(&root, &outside));

        // Non-existent leaf under an existing root passes its parent check.
        let ghost = root.join("missing.txt");
        assert!(path_within_canonical(&root, &ghost));
    }
}
