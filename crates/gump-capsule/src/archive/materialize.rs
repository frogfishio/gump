//! Local Capsule application materialization (DELIVERY F06 / FORMATS.md §6).
//!
//! Target layout: `<state-root>/apps/<capsule-id>/` via exclusive staging +
//! atomic no-replace rename (STL-06).

use std::fs;
use std::io::ErrorKind;
use std::io::Read;
use std::path::{Path, PathBuf};

use gump_types::CapsuleId;

use super::error::{ArchiveError, ArchiveErrorKind};
use super::extract::{ExtractLimits, extract_ustar_zstd_from_reader};

/// Result of materializing an application archive into the local apps cache.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedRelease {
    pub root: PathBuf,
    pub capsule_id: CapsuleId,
    pub file_count: usize,
}

/// Materialize `ustar+zstd/1` bytes under `state_root/apps/<capsule-id>/`.
///
/// Extraction happens in an exclusive random staging directory beside the
/// target, then an atomic rename publishes the tree. On failure only this
/// operation's staging dir is removed — never the published target (STL-06).
///
/// `archive_zst` is a streaming reader — the API does not require a complete
/// in-memory archive slice (STL-14 / CONFORMANCE).
pub fn materialize_application_archive<R: Read>(
    state_root: &Path,
    capsule_id: CapsuleId,
    archive_zst: R,
    limits: &ExtractLimits,
) -> Result<MaterializedRelease, ArchiveError> {
    let apps = state_root.join("apps");
    fs::create_dir_all(&apps)?;
    let target = apps.join(capsule_id.to_hyphenated());
    if target.exists() {
        return Err(ArchiveError::new(
            ArchiveErrorKind::Io,
            format!("materialization already exists at {}", target.display()),
        ));
    }

    let staging = create_exclusive_staging(&apps)?;
    let result = (|| {
        let file_count = extract_ustar_zstd_from_reader(archive_zst, &staging, limits)?;
        // No-replace publish: rename fails if another winner already occupies target.
        publish_no_replace(&staging, &target)?;
        Ok(MaterializedRelease {
            root: target.clone(),
            capsule_id,
            file_count,
        })
    })();

    if result.is_err() {
        // STL-06: cleanup only dirs owned by this op — never unlink `target`.
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

/// Create a unique staging directory under `apps` (create-new; no shared name).
fn create_exclusive_staging(apps: &Path) -> Result<PathBuf, ArchiveError> {
    for _ in 0..64 {
        // Random v7 id — not derivable from capsule id + pid (STL-06).
        let name = format!(".staging-{}", CapsuleId::new().to_hyphenated());
        let staging = apps.join(&name);
        match fs::create_dir(&staging) {
            Ok(()) => return Ok(staging),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(ArchiveError::new(
        ArchiveErrorKind::Io,
        "exhausted exclusive staging directory attempts",
    ))
}

/// Atomically publish `staging` → `target` without replacing an existing tree.
///
/// Linux: `renameat2(..., RENAME_NOREPLACE)`. Apple: `renameatx_np(..., RENAME_EXCL)`.
/// Ordinary `rename` is not sufficient — POSIX allows replacing an empty directory (STL-17).
fn publish_no_replace(staging: &Path, target: &Path) -> Result<(), ArchiveError> {
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "redox"
    ))]
    {
        publish_no_replace_atomic(staging, target)
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "redox"
    )))]
    {
        publish_no_replace_fallback(staging, target)
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    target_os = "redox"
))]
fn publish_no_replace_atomic(staging: &Path, target: &Path) -> Result<(), ArchiveError> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    use rustix::io::Errno;

    match renameat_with(CWD, staging, CWD, target, RenameFlags::NOREPLACE) {
        Ok(()) => Ok(()),
        Err(err) if err == Errno::EXIST || err == Errno::NOTEMPTY => Err(ArchiveError::new(
            ArchiveErrorKind::Io,
            format!("materialization already exists at {}", target.display()),
        )),
        Err(err) => Err(ArchiveError::new(
            ArchiveErrorKind::Io,
            format!("atomic materialize rename failed: {err}"),
        )),
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    target_os = "redox"
)))]
fn publish_no_replace_fallback(staging: &Path, target: &Path) -> Result<(), ArchiveError> {
    if target.exists() {
        return Err(ArchiveError::new(
            ArchiveErrorKind::Io,
            format!("materialization already exists at {}", target.display()),
        ));
    }
    fs::rename(staging, target).map_err(|e| {
        ArchiveError::new(
            ArchiveErrorKind::Io,
            format!("atomic materialize rename failed: {e}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("gump-stl17-{name}-{nanos}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn publish_no_replace_refuses_empty_destination() {
        let root = tmp("empty-dest");
        let staging = root.join("staging");
        let target = root.join("target");
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("marker"), b"winner-bytes").unwrap();
        // Empty destination — ordinary rename may replace this; NOREPLACE must not.
        fs::create_dir(&target).unwrap();

        let err = publish_no_replace(&staging, &target).unwrap_err();
        assert_eq!(err.kind(), ArchiveErrorKind::Io);
        assert!(
            target.is_dir() && !target.join("marker").exists(),
            "empty destination must not be replaced"
        );
        assert!(
            staging.join("marker").exists(),
            "failed publish must leave staging intact for caller cleanup"
        );
        let _ = fs::remove_dir_all(root);
    }
}
