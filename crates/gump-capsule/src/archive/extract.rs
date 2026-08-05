//! Safe materialization of archive entries into a private staging directory.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::error::{ArchiveError, ArchiveErrorKind};
use super::pack::{ArchiveEntry, EntryKind};
use super::path::validate_archive_path;

/// Ceilings applied during unpack/extract (FORMATS.md §6).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractLimits {
    pub max_files: usize,
    pub max_uncompressed_bytes: u64,
    pub max_path_bytes: usize,
}

impl Default for ExtractLimits {
    fn default() -> Self {
        Self {
            max_files: 100_000,
            max_uncompressed_bytes: 512 * 1024 * 1024,
            max_path_bytes: 4_096,
        }
    }
}

/// Extract entries under `staging_root`, rejecting escapes and overwrites of symlinks.
///
/// `staging_root` must already exist. No entry may resolve outside it.
pub fn extract_entries(
    staging_root: &Path,
    entries: &[ArchiveEntry],
    limits: &ExtractLimits,
) -> Result<(), ArchiveError> {
    let root = staging_root
        .canonicalize()
        .map_err(|e| ArchiveError::new(ArchiveErrorKind::Io, e.to_string()))?;
    if entries.len() > limits.max_files {
        return Err(ArchiveError::new(
            ArchiveErrorKind::Limit,
            "file count ceiling exceeded",
        ));
    }
    let mut total = 0u64;
    for entry in entries {
        if entry.path.as_bytes().len() > limits.max_path_bytes {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Limit,
                "path length ceiling exceeded",
            ));
        }
        let path = validate_archive_path(
            entry.path.trim_end_matches('/'),
            entry.kind == EntryKind::Directory,
        )?;
        if path != entry.path {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Path,
                "entry path not normalized",
            ));
        }
        total = total.saturating_add(entry.data.len() as u64);
        if total > limits.max_uncompressed_bytes {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Limit,
                "expanded size ceiling exceeded",
            ));
        }
        let rel = path.trim_end_matches('/');
        let dest = join_jail(&root, rel)?;
        match entry.kind {
            EntryKind::Directory => {
                fs::create_dir_all(&dest)?;
                deny_symlink(&dest)?;
            }
            EntryKind::File => {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                    deny_symlink(parent)?;
                }
                // Refuse to follow a pre-existing symlink at the destination.
                if dest.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false)
                {
                    return Err(ArchiveError::new(
                        ArchiveErrorKind::Escape,
                        format!("refusing to write through symlink {}", dest.display()),
                    ));
                }
                let mut f = fs::File::create(&dest)?;
                f.write_all(&entry.data)?;
                f.sync_all()?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = if entry.executable { 0o755 } else { 0o644 };
                    fs::set_permissions(&dest, fs::Permissions::from_mode(mode))?;
                }
                deny_symlink(&dest)?;
            }
        }
    }
    Ok(())
}

fn join_jail(root: &Path, rel: &str) -> Result<PathBuf, ArchiveError> {
    let mut out = root.to_path_buf();
    for seg in rel.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Escape,
                "path escape during extract",
            ));
        }
        out.push(seg);
    }
    // Lexical jail: every intermediate must stay under root when string-prefixed
    // after join; canonicalize parents that exist.
    if let Some(parent) = out.parent() {
        if parent.exists() {
            let parent_canon = parent.canonicalize().map_err(|e| {
                ArchiveError::new(ArchiveErrorKind::Io, e.to_string())
            })?;
            if !parent_canon.starts_with(root) {
                return Err(ArchiveError::new(
                    ArchiveErrorKind::Escape,
                    "extract path escapes staging root",
                ));
            }
        }
    }
    Ok(out)
}

fn deny_symlink(path: &Path) -> Result<(), ArchiveError> {
    let meta = match path.symlink_metadata() {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    if meta.file_type().is_symlink() {
        return Err(ArchiveError::new(
            ArchiveErrorKind::Escape,
            format!("symlink rejected at {}", path.display()),
        ));
    }
    Ok(())
}
