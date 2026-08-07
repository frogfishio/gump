//! Local Capsule application materialization (DELIVERY F06 / FORMATS.md §6).
//!
//! Target layout: `<state-root>/apps/<capsule-id>/` via exclusive staging +
//! atomic no-replace rename (STL-06).

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use gump_types::CapsuleId;

use super::error::{ArchiveError, ArchiveErrorKind};
use super::extract::{ExtractLimits, extract_entries};
use super::pack::unpack_archive;

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
pub fn materialize_application_archive(
    state_root: &Path,
    capsule_id: CapsuleId,
    archive_zst: &[u8],
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
        let entries = unpack_archive(archive_zst, limits)?;
        extract_entries(&staging, &entries, limits)?;
        // No-replace publish: rename fails if another winner already occupies target.
        publish_no_replace(&staging, &target)?;
        Ok(MaterializedRelease {
            root: target.clone(),
            capsule_id,
            file_count: entries.len(),
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
fn publish_no_replace(staging: &Path, target: &Path) -> Result<(), ArchiveError> {
    if target.exists() {
        return Err(ArchiveError::new(
            ArchiveErrorKind::Io,
            format!("materialization already exists at {}", target.display()),
        ));
    }
    fs::rename(staging, target).map_err(|e| {
        // Concurrent winner: destination appeared between exists() and rename.
        ArchiveError::new(
            ArchiveErrorKind::Io,
            format!("atomic materialize rename failed: {e}"),
        )
    })
}
