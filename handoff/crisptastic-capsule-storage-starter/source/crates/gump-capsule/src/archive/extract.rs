//! Safe materialization of archive entries into a private staging directory.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::error::{ArchiveError, ArchiveErrorKind};
use super::pack::{ArchiveEntry, EntryKind};
use super::path::validate_archive_path;
use super::ustar::parse_header;

const BLOCK: usize = 512;

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
        if entry.path.len() > limits.max_path_bytes {
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
                if dest
                    .symlink_metadata()
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false)
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

/// Decompress a `ustar+zstd/1` stream and extract entry-by-entry into `staging_root`.
///
/// Never buffers the full uncompressed archive or all entry bodies (STL-14).
/// Returns the number of entries extracted.
pub fn extract_ustar_zstd_from_reader<R: Read>(
    reader: R,
    staging_root: &Path,
    limits: &ExtractLimits,
) -> Result<usize, ArchiveError> {
    let mut decoder = zstd::stream::Decoder::new(reader)
        .map_err(|e| ArchiveError::new(ArchiveErrorKind::Compress, e.to_string()))?;
    extract_ustar_from_reader(&mut decoder, staging_root, limits)
}

/// Parse a Gump-normalized ustar byte stream from `reader`, writing files as they appear.
pub fn extract_ustar_from_reader<R: Read>(
    reader: &mut R,
    staging_root: &Path,
    limits: &ExtractLimits,
) -> Result<usize, ArchiveError> {
    let root = staging_root
        .canonicalize()
        .map_err(|e| ArchiveError::new(ArchiveErrorKind::Io, e.to_string()))?;
    let mut file_count = 0usize;
    let mut expanded = 0u64;
    let mut header = [0u8; BLOCK];

    loop {
        read_exact(reader, &mut header)?;
        if header.iter().all(|&b| b == 0) {
            // End marker: require a second zero block.
            read_exact(reader, &mut header)?;
            if !header.iter().all(|&b| b == 0) {
                return Err(ArchiveError::new(
                    ArchiveErrorKind::Format,
                    "ustar end marker missing second zero block",
                ));
            }
            break;
        }

        let (path, kind, executable, size) = parse_header(&header)?;
        if file_count >= limits.max_files {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Limit,
                "file count ceiling exceeded",
            ));
        }
        if path.len() > limits.max_path_bytes {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Limit,
                "path length ceiling exceeded",
            ));
        }
        expanded = expanded.saturating_add(size);
        if expanded > limits.max_uncompressed_bytes {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Limit,
                "expanded size ceiling exceeded",
            ));
        }

        let rel = path.trim_end_matches('/');
        let dest = join_jail(&root, rel)?;
        match kind {
            EntryKind::Directory => {
                if size != 0 {
                    return Err(ArchiveError::new(
                        ArchiveErrorKind::Format,
                        "directory entry has payload",
                    ));
                }
                fs::create_dir_all(&dest)?;
                deny_symlink(&dest)?;
            }
            EntryKind::File => {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                    deny_symlink(parent)?;
                }
                if dest
                    .symlink_metadata()
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false)
                {
                    return Err(ArchiveError::new(
                        ArchiveErrorKind::Escape,
                        format!("refusing to write through symlink {}", dest.display()),
                    ));
                }
                let mut f = fs::File::create(&dest)?;
                copy_exact(reader, &mut f, size)?;
                f.sync_all()?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = if executable { 0o755 } else { 0o644 };
                    fs::set_permissions(&dest, fs::Permissions::from_mode(mode))?;
                }
                deny_symlink(&dest)?;
                let pad = (BLOCK as u64 - (size % BLOCK as u64)) % BLOCK as u64;
                discard_exact(reader, pad)?;
            }
        }
        file_count = file_count.saturating_add(1);
    }

    // Reject trailing payload after end markers (match buffered parse_ustar).
    let mut extra = [0u8; 1];
    match reader.read(&mut extra) {
        Ok(0) => Ok(file_count),
        Ok(_) => Err(ArchiveError::new(
            ArchiveErrorKind::Format,
            "trailing data after ustar end markers",
        )),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(file_count),
        Err(e) => Err(ArchiveError::new(ArchiveErrorKind::Io, e.to_string())),
    }
}

fn read_exact<R: Read>(reader: &mut R, buf: &mut [u8]) -> Result<(), ArchiveError> {
    reader
        .read_exact(buf)
        .map_err(|e| ArchiveError::new(ArchiveErrorKind::Format, e.to_string()))
}

fn copy_exact<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    mut remaining: u64,
) -> Result<(), ArchiveError> {
    let mut buf = [0u8; 64 * 1024];
    while remaining > 0 {
        let n = (remaining as usize).min(buf.len());
        read_exact(reader, &mut buf[..n])?;
        writer
            .write_all(&buf[..n])
            .map_err(|e| ArchiveError::new(ArchiveErrorKind::Io, e.to_string()))?;
        remaining -= n as u64;
    }
    Ok(())
}

fn discard_exact<R: Read>(reader: &mut R, mut remaining: u64) -> Result<(), ArchiveError> {
    let mut buf = [0u8; BLOCK];
    while remaining > 0 {
        let n = (remaining as usize).min(buf.len());
        read_exact(reader, &mut buf[..n])?;
        if buf[..n].iter().any(|&b| b != 0) {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Format,
                "non-zero ustar padding",
            ));
        }
        remaining -= n as u64;
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
            let parent_canon = parent
                .canonicalize()
                .map_err(|e| ArchiveError::new(ArchiveErrorKind::Io, e.to_string()))?;
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
