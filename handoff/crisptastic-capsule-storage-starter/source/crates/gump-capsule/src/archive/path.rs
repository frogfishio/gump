//! Archive path rules (FORMATS.md §6).

use unicode_normalization::{UnicodeNormalization, is_nfc};

use super::error::{ArchiveError, ArchiveErrorKind};

/// Validate and normalize a relative archive path (NFC, `/`, no `.`/`..`).
///
/// Directory paths MUST end with `/`. Regular-file paths MUST NOT.
pub fn validate_archive_path(path: &str, directory: bool) -> Result<String, ArchiveError> {
    if path.is_empty() {
        return Err(ArchiveError::new(ArchiveErrorKind::Path, "empty path"));
    }
    if path.starts_with('/') {
        return Err(ArchiveError::new(
            ArchiveErrorKind::Escape,
            "absolute path rejected",
        ));
    }
    if path.contains('\\') || path.contains('\0') {
        return Err(ArchiveError::new(
            ArchiveErrorKind::Path,
            "invalid path character",
        ));
    }
    if !directory && path.ends_with('/') {
        return Err(ArchiveError::new(
            ArchiveErrorKind::Path,
            "regular file path must not end with '/'",
        ));
    }

    let nfc: String = if is_nfc(path) {
        path.to_string()
    } else {
        path.nfc().collect()
    };

    let body = nfc.trim_end_matches('/');
    if body.is_empty() {
        return Err(ArchiveError::new(ArchiveErrorKind::Path, "empty path"));
    }

    let mut segments = Vec::new();
    for seg in body.split('/') {
        if seg.is_empty() {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Path,
                "empty path segment rejected",
            ));
        }
        if seg == "." || seg == ".." {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Escape,
                format!("path segment {seg:?} rejected"),
            ));
        }
        segments.push(seg);
    }

    let mut out = segments.join("/");
    if directory {
        out.push('/');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_relative_file_and_dir() {
        assert_eq!(
            validate_archive_path("bin/hello", false).unwrap(),
            "bin/hello"
        );
        assert_eq!(validate_archive_path("bin", true).unwrap(), "bin/");
        assert_eq!(validate_archive_path("bin/", true).unwrap(), "bin/");
    }

    #[test]
    fn rejects_escape() {
        assert_eq!(
            validate_archive_path("../x", false).unwrap_err().kind(),
            ArchiveErrorKind::Escape
        );
        assert_eq!(
            validate_archive_path("/etc/passwd", false)
                .unwrap_err()
                .kind(),
            ArchiveErrorKind::Escape
        );
        assert_eq!(
            validate_archive_path("a//b", false).unwrap_err().kind(),
            ArchiveErrorKind::Path
        );
    }
}
