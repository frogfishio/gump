//! Virtual package tree + two-pass SOURCE_CHANGED capture.
//!
//! STL-05: capture retains immutable bytes from no-follow opens. Pack/CLI must
//! archive those bytes (re-verify digest/len); never re-open workspace paths.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::model::PrepareOutput;

use super::deny::is_sensitive_relative_path;
use super::plan::{CaptureError, CaptureErrorKind, CapturePlan};

/// Stable identity used to detect races between metadata passes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileIdentity {
    pub len: u64,
    pub modified: Option<SystemTime>,
    pub digest: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VirtualEntry {
    pub relative_path: String,
    pub identity: FileIdentity,
    /// Exact bytes hashed into `identity.digest` (FORMATS §11 / STL-05).
    pub bytes: Vec<u8>,
    /// Executable bit observed on the no-follow open (unix); false elsewhere.
    pub executable: bool,
    /// Absolute path of the captured source (provenance only — do not re-read).
    pub source_path: PathBuf,
    pub from_prepare: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VirtualTree {
    entries: BTreeMap<String, VirtualEntry>,
}

impl VirtualTree {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, rel: &str) -> Option<&VirtualEntry> {
        self.entries.get(rel)
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(|s| s.as_str())
    }

    pub fn insert(&mut self, entry: VirtualEntry) {
        self.entries.insert(entry.relative_path.clone(), entry);
    }
}

/// Verify retained bytes still match the recorded identity (pack-time check).
pub fn verify_captured_bytes(entry: &VirtualEntry) -> Result<(), CaptureError> {
    if entry.bytes.len() as u64 != entry.identity.len {
        return Err(CaptureError::new(
            CaptureErrorKind::SourceChanged,
            format!(
                "SOURCE_CHANGED: length drift for {} ({} vs {})",
                entry.relative_path,
                entry.bytes.len(),
                entry.identity.len
            ),
        ));
    }
    let digest = *blake3::hash(&entry.bytes).as_bytes();
    if digest != entry.identity.digest {
        return Err(CaptureError::new(
            CaptureErrorKind::SourceChanged,
            format!("SOURCE_CHANGED: digest drift for {}", entry.relative_path),
        ));
    }
    Ok(())
}

/// Capture allowlisted files under `workspace_root` into a virtual tree.
///
/// Performs two identity passes; any change yields `SOURCE_CHANGED`.
/// The confirming pass's bytes are retained for pack (STL-05).
pub fn capture_workspace(
    workspace_root: &Path,
    plan: &CapturePlan,
) -> Result<VirtualTree, CaptureError> {
    let root = workspace_root
        .canonicalize()
        .map_err(|e| CaptureError::new(CaptureErrorKind::Io, e.to_string()))?;

    let first = scan_once(&root, plan)?;
    let second = scan_once(&root, plan)?;
    if !pass_identities_match(&first, &second) {
        return Err(CaptureError::new(
            CaptureErrorKind::SourceChanged,
            "SOURCE_CHANGED: workspace mutated during capture",
        ));
    }

    let mut tree = VirtualTree::default();
    for (rel, blob) in second {
        let source_path = root.join(&rel);
        ensure_within_root(&root, &source_path)?;
        tree.insert(VirtualEntry {
            relative_path: rel,
            identity: blob.identity,
            bytes: blob.bytes,
            executable: blob.executable,
            source_path,
            from_prepare: false,
        });
    }
    Ok(tree)
}

/// Copy prepare outputs into the virtual tree under declared `to` paths.
///
/// Does not execute prepare commands — callers stage outputs first. Paths are
/// jail-checked; sensitive targets still require `allow_sensitive_files`.
pub fn apply_prepare_outputs(
    workspace_root: &Path,
    tree: &mut VirtualTree,
    staging_root: &Path,
    outputs: &[PrepareOutput],
    allow_sensitive_files: bool,
) -> Result<(), CaptureError> {
    let root = workspace_root
        .canonicalize()
        .map_err(|e| CaptureError::new(CaptureErrorKind::Io, e.to_string()))?;
    let staging = staging_root
        .canonicalize()
        .map_err(|e| CaptureError::new(CaptureErrorKind::Io, e.to_string()))?;

    for out in outputs {
        let rel = normalize_rel(&out.to)?;
        maybe_deny_sensitive(&rel, allow_sensitive_files)?;
        let from = staging.join(&out.from);
        ensure_within_root(&staging, &from)?;
        let blob = read_regular_nofollow(&from)?;
        let blob2 = read_regular_nofollow(&from)?;
        if blob.identity != blob2.identity || blob.executable != blob2.executable {
            return Err(CaptureError::new(
                CaptureErrorKind::SourceChanged,
                format!("SOURCE_CHANGED: prepare output {}", out.from),
            ));
        }
        let _ = &root; // jail context for package root documentation
        tree.insert(VirtualEntry {
            relative_path: rel,
            identity: blob2.identity,
            bytes: blob2.bytes,
            executable: blob2.executable,
            source_path: from,
            from_prepare: true,
        });
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CapturedBlob {
    identity: FileIdentity,
    bytes: Vec<u8>,
    executable: bool,
}

fn pass_identities_match(
    a: &BTreeMap<String, CapturedBlob>,
    b: &BTreeMap<String, CapturedBlob>,
) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().all(|(k, va)| {
        b.get(k)
            .map(|vb| va.identity == vb.identity && va.executable == vb.executable)
            .unwrap_or(false)
    })
}

fn scan_once(
    root: &Path,
    plan: &CapturePlan,
) -> Result<BTreeMap<String, CapturedBlob>, CaptureError> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        ensure_within_root(root, &dir)?;
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                return Err(CaptureError::new(
                    CaptureErrorKind::Escape,
                    format!("symlink rejected: {}", display_rel(root, &path)),
                ));
            }
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                return Err(CaptureError::new(
                    CaptureErrorKind::Escape,
                    format!("non-regular file rejected: {}", display_rel(root, &path)),
                ));
            }
            let rel = display_rel(root, &path);
            let rel = normalize_rel(&rel)?;
            if !plan.matches(&rel) {
                continue;
            }
            maybe_deny_sensitive(&rel, plan.allow_sensitive_files)?;
            ensure_within_root(root, &path)?;
            out.insert(rel, read_regular_nofollow(&path)?);
        }
    }
    Ok(out)
}

fn maybe_deny_sensitive(rel: &str, allow_sensitive: bool) -> Result<(), CaptureError> {
    match is_sensitive_relative_path(rel)? {
        Some(reason) if !allow_sensitive => Err(CaptureError::new(
            CaptureErrorKind::Sensitive,
            format!("{} denied ({})", rel, reason.as_str()),
        )),
        _ => Ok(()),
    }
}

fn normalize_rel(rel: &str) -> Result<String, CaptureError> {
    // Force forward slashes and validate components via sensitive check.
    let rel = rel.replace('\\', "/");
    let _ = is_sensitive_relative_path(&rel).map_err(CaptureError::from)?;
    // Re-run component walk to produce cleaned path.
    let path = Path::new(&rel);
    let mut parts = Vec::new();
    for comp in path.components() {
        match comp {
            std::path::Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            std::path::Component::CurDir => {}
            _ => {
                return Err(CaptureError::new(
                    CaptureErrorKind::Escape,
                    format!("invalid relative path {rel:?}"),
                ));
            }
        }
    }
    if parts.is_empty() {
        return Err(CaptureError::new(
            CaptureErrorKind::Escape,
            "empty relative path",
        ));
    }
    Ok(parts.join("/"))
}

fn ensure_within_root(root: &Path, path: &Path) -> Result<(), CaptureError> {
    let canon = if path.exists() {
        path.canonicalize()
            .map_err(|e| CaptureError::new(CaptureErrorKind::Io, e.to_string()))?
    } else {
        // For not-yet-created paths, canonicalize parent + join name.
        let parent = path
            .parent()
            .ok_or_else(|| CaptureError::new(CaptureErrorKind::Escape, "path has no parent"))?;
        let name = path
            .file_name()
            .ok_or_else(|| CaptureError::new(CaptureErrorKind::Escape, "path has no file name"))?;
        parent
            .canonicalize()
            .map_err(|e| CaptureError::new(CaptureErrorKind::Io, e.to_string()))?
            .join(name)
    };
    if !canon.starts_with(root) {
        return Err(CaptureError::new(
            CaptureErrorKind::Escape,
            format!("path escapes workspace root: {}", canon.display()),
        ));
    }
    Ok(())
}

fn display_rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"))
}

/// Open a regular file without following symlinks; hash bytes from that fd.
fn read_regular_nofollow(path: &Path) -> Result<CapturedBlob, CaptureError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let mut opts = fs::OpenOptions::new();
        opts.read(true);
        // FORMATS §11 / STL-05: O_NOFOLLOW — symlink swap cannot redirect the open.
        opts.custom_flags(libc::O_NOFOLLOW);
        let mut file = opts.open(path).map_err(|e| {
            // ELOOP when the final component is a symlink.
            if e.raw_os_error() == Some(libc::ELOOP) {
                CaptureError::new(
                    CaptureErrorKind::Escape,
                    format!("symlink rejected at open: {}", path.display()),
                )
            } else {
                CaptureError::new(CaptureErrorKind::Io, e.to_string())
            }
        })?;
        let meta = file
            .metadata()
            .map_err(|e| CaptureError::new(CaptureErrorKind::Io, e.to_string()))?;
        if !meta.file_type().is_file() {
            return Err(CaptureError::new(
                CaptureErrorKind::Escape,
                format!("non-regular file rejected: {}", path.display()),
            ));
        }
        let executable = meta.permissions().mode() & 0o111 != 0;
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        file.read_to_end(&mut bytes)
            .map_err(|e| CaptureError::new(CaptureErrorKind::Io, e.to_string()))?;
        if bytes.len() as u64 != meta.len() {
            return Err(CaptureError::new(
                CaptureErrorKind::SourceChanged,
                format!(
                    "SOURCE_CHANGED: size changed while reading {}",
                    path.display()
                ),
            ));
        }
        let digest = *blake3::hash(&bytes).as_bytes();
        Ok(CapturedBlob {
            identity: FileIdentity {
                len: meta.len(),
                modified: meta.modified().ok(),
                digest,
            },
            bytes,
            executable,
        })
    }
    #[cfg(not(unix))]
    {
        let meta = fs::symlink_metadata(path)
            .map_err(|e| CaptureError::new(CaptureErrorKind::Io, e.to_string()))?;
        if meta.file_type().is_symlink() {
            return Err(CaptureError::new(
                CaptureErrorKind::Escape,
                format!("symlink rejected: {}", path.display()),
            ));
        }
        if !meta.file_type().is_file() {
            return Err(CaptureError::new(
                CaptureErrorKind::Escape,
                format!("non-regular file rejected: {}", path.display()),
            ));
        }
        let bytes =
            fs::read(path).map_err(|e| CaptureError::new(CaptureErrorKind::Io, e.to_string()))?;
        if bytes.len() as u64 != meta.len() {
            return Err(CaptureError::new(
                CaptureErrorKind::SourceChanged,
                format!(
                    "SOURCE_CHANGED: size changed while reading {}",
                    path.display()
                ),
            ));
        }
        let digest = *blake3::hash(&bytes).as_bytes();
        Ok(CapturedBlob {
            identity: FileIdentity {
                len: meta.len(),
                modified: meta.modified().ok(),
                digest,
            },
            bytes,
            executable: false,
        })
    }
}
