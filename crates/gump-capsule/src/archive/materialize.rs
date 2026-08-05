//! Local Capsule application materialization (DELIVERY F06 / FORMATS.md §6).
//!
//! Target layout: `<state-root>/apps/<capsule-id>/` via staging + atomic rename.

use std::fs;
use std::path::{Path, PathBuf};

use gump_types::CapsuleId;

use super::error::{ArchiveError, ArchiveErrorKind};
use super::extract::{extract_entries, ExtractLimits};
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
/// Extraction happens in a private staging directory beside the target, then an
/// atomic rename publishes the tree. Symlink escapes and bomb ceilings are
/// enforced by the archive extractor.
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
            format!(
                "materialization already exists at {}",
                target.display()
            ),
        ));
    }

    let staging_name = format!(
        ".staging-{}-{}",
        capsule_id.to_hyphenated(),
        std::process::id()
    );
    let staging = apps.join(&staging_name);
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;

    let result = (|| {
        let entries = unpack_archive(archive_zst, limits)?;
        extract_entries(&staging, &entries, limits)?;
        // Publish: rename staging → target (atomic on same filesystem).
        fs::rename(&staging, &target).map_err(|e| {
            ArchiveError::new(
                ArchiveErrorKind::Io,
                format!("atomic materialize rename failed: {e}"),
            )
        })?;
        Ok(MaterializedRelease {
            root: target.clone(),
            capsule_id,
            file_count: entries.len(),
        })
    })();

    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
        let _ = fs::remove_dir_all(&target);
    }
    result
}
