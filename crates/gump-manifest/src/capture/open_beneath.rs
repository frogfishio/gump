//! Root-handle opens for capture (STL-16).
//!
//! Linux: `openat2` with `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS`.
//! Other Unix: descriptor-relative `openat` walk with `O_NOFOLLOW` on every
//! component so an intermediate directory cannot be swapped to a symlink
//! between validation and the final open (FORMATS §11 / STL-05 residual).

use std::fs::File;
use std::io::Read;
use std::path::Path;

use super::plan::{CaptureError, CaptureErrorKind};
use super::tree::{CapturedBlob, FileIdentity};

/// Open `rel` under `root` without following any symlink component; return bytes.
pub(super) fn read_regular_beneath(root: &Path, rel: &str) -> Result<CapturedBlob, CaptureError> {
    let components = rel_components(rel)?;
    #[cfg(unix)]
    {
        read_regular_beneath_unix(root, &components, rel)
    }
    #[cfg(not(unix))]
    {
        let path = root.join(rel);
        read_regular_nofollow_fallback(&path)
    }
}

fn rel_components(rel: &str) -> Result<Vec<&str>, CaptureError> {
    let mut parts = Vec::new();
    for part in rel.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains('\0') {
            return Err(CaptureError::new(
                CaptureErrorKind::Escape,
                format!("invalid relative path {rel:?}"),
            ));
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return Err(CaptureError::new(
            CaptureErrorKind::Escape,
            "empty relative path",
        ));
    }
    Ok(parts)
}

#[cfg(unix)]
fn read_regular_beneath_unix(
    root: &Path,
    components: &[&str],
    rel: &str,
) -> Result<CapturedBlob, CaptureError> {
    use rustix::fs::{Mode, OFlags, open};
    use std::os::unix::fs::PermissionsExt;

    let root_fd = open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| map_open_err(e, rel))?;

    let file_fd = open_leaf_beneath(&root_fd, components, rel)?;
    let mut file = File::from(file_fd);
    let meta = file
        .metadata()
        .map_err(|e| CaptureError::new(CaptureErrorKind::Io, e.to_string()))?;
    if !meta.file_type().is_file() {
        return Err(CaptureError::new(
            CaptureErrorKind::Escape,
            format!("non-regular file rejected: {rel}"),
        ));
    }
    let executable = meta.permissions().mode() & 0o111 != 0;
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.read_to_end(&mut bytes)
        .map_err(|e| CaptureError::new(CaptureErrorKind::Io, e.to_string()))?;
    if bytes.len() as u64 != meta.len() {
        return Err(CaptureError::new(
            CaptureErrorKind::SourceChanged,
            format!("SOURCE_CHANGED: size changed while reading {rel}"),
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

#[cfg(all(unix, target_os = "linux"))]
fn open_leaf_beneath(
    root_fd: &impl std::os::fd::AsFd,
    components: &[&str],
    rel: &str,
) -> Result<rustix::fd::OwnedFd, CaptureError> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};

    // Single openat2 of the full relative path — no intermediate path reopen.
    let joined = components.join("/");
    openat2(
        root_fd,
        joined.as_str(),
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS,
    )
    .map_err(|e| map_open_err(e, rel))
}

#[cfg(all(unix, not(target_os = "linux")))]
fn open_leaf_beneath(
    root_fd: &impl std::os::fd::AsFd,
    components: &[&str],
    rel: &str,
) -> Result<rustix::fd::OwnedFd, CaptureError> {
    use rustix::fs::{Mode, OFlags, openat};

    let mut dir = None::<rustix::fd::OwnedFd>;
    let last = components.len() - 1;
    for (i, name) in components.iter().enumerate() {
        let parent: &dyn std::os::fd::AsFd = match &dir {
            Some(fd) => fd,
            None => root_fd,
        };
        let flags = if i == last {
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW
        } else {
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::DIRECTORY
        };
        let next = openat(parent, *name, flags, Mode::empty()).map_err(|e| map_open_err(e, rel))?;
        if i == last {
            return Ok(next);
        }
        dir = Some(next);
    }
    Err(CaptureError::new(
        CaptureErrorKind::Escape,
        format!("empty relative path: {rel}"),
    ))
}

#[cfg(unix)]
fn map_open_err(err: rustix::io::Errno, rel: &str) -> CaptureError {
    use rustix::io::Errno;
    // Symlink / resolve-beneath failures fail closed as Escape.
    if err == Errno::LOOP || err == Errno::XDEV || err == Errno::NOTDIR || err == Errno::INVAL {
        CaptureError::new(
            CaptureErrorKind::Escape,
            format!("symlink or escape rejected at open: {rel} ({err})"),
        )
    } else {
        CaptureError::new(CaptureErrorKind::Io, format!("open {rel}: {err}"))
    }
}

#[cfg(not(unix))]
fn read_regular_nofollow_fallback(path: &Path) -> Result<CapturedBlob, CaptureError> {
    use std::fs;

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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    fn tmp(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "gump-manifest-stl16-{}-{}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn nested_file_opens_beneath_root() {
        let root = tmp("nested-ok");
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::write(root.join("a/b/hello"), b"payload").unwrap();
        let blob = read_regular_beneath(&root, "a/b/hello").unwrap();
        assert_eq!(blob.bytes, b"payload");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn intermediate_dir_symlink_fails_closed() {
        // STL-16: intermediate component is a symlink — must not follow to host.
        let root = tmp("mid-symlink");
        fs::create_dir_all(root.join("via")).unwrap();
        symlink("/etc", root.join("via/mid")).unwrap();
        let err = read_regular_beneath(&root, "via/mid/passwd").unwrap_err();
        assert_eq!(err.kind(), CaptureErrorKind::Escape);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn final_symlink_fails_closed() {
        let root = tmp("final-symlink");
        fs::create_dir_all(root.join("bin")).unwrap();
        symlink("/etc/passwd", root.join("bin/hello")).unwrap();
        let err = read_regular_beneath(&root, "bin/hello").unwrap_err();
        assert_eq!(err.kind(), CaptureErrorKind::Escape);
        let _ = fs::remove_dir_all(root);
    }
}
