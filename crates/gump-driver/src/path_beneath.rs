//! Descriptor-style path containment under a release/attempt root (GUMP-N001).
//!
//! Rejects absolute paths, `..`, NUL, and any symlink component so runtime
//! command/workdir/cleanup cannot escape the verified release or owned attempt
//! root (F06 / R06 / INV-002 / INV-014).
//!
//! Walk uses `symlink_metadata` per component (no follow). Residual TOCTOU
//! between prepare and spawn is narrowed by re-checking at start.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::{DriverError, DriverErrorKind};

/// Split a release-relative path into safe components (no `..`, absolute, or NUL).
pub(crate) fn rel_components(rel: &str) -> Result<Vec<&str>, DriverError> {
    if rel.is_empty() {
        return Err(DriverError::new(
            DriverErrorKind::Policy,
            "empty relative path",
        ));
    }
    if rel.as_bytes().contains(&0) {
        return Err(DriverError::new(
            DriverErrorKind::Policy,
            "NUL in relative path",
        ));
    }
    if Path::new(rel).is_absolute() {
        return Err(DriverError::new(
            DriverErrorKind::Policy,
            format!("absolute path rejected: {rel}"),
        ));
    }
    let mut parts = Vec::new();
    for part in rel.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(DriverError::new(
                DriverErrorKind::Policy,
                format!("parent traversal rejected: {rel}"),
            ));
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return Err(DriverError::new(
            DriverErrorKind::Policy,
            format!("empty relative path after normalize: {rel}"),
        ));
    }
    Ok(parts)
}

/// Resolve `rel` under `root` without following any symlink component.
///
/// Final path must exist and match `expect` (file or directory).
pub(crate) fn resolve_beneath(
    root: &Path,
    rel: &str,
    expect: PathKind,
) -> Result<PathBuf, DriverError> {
    let components = rel_components(rel)?;
    if !root.is_dir() {
        return Err(DriverError::new(
            DriverErrorKind::Prepare,
            "containment root is not a directory",
        ));
    }
    // Root itself must not be a symlink (escape via release root swap).
    let root_meta = fs::symlink_metadata(root).map_err(|e| {
        DriverError::new(
            DriverErrorKind::Io,
            format!("stat release root {}: {e}", root.display()),
        )
    })?;
    if root_meta.file_type().is_symlink() {
        return Err(DriverError::new(
            DriverErrorKind::Policy,
            "containment root must not be a symlink",
        ));
    }

    let mut cur = root.to_path_buf();
    for (i, part) in components.iter().enumerate() {
        let next = cur.join(part);
        let meta = fs::symlink_metadata(&next).map_err(|e| {
            DriverError::new(
                DriverErrorKind::NotFound,
                format!("path not found under root ({rel}): {e}"),
            )
        })?;
        if meta.file_type().is_symlink() {
            return Err(DriverError::new(
                DriverErrorKind::Policy,
                format!("symlink component rejected in {rel:?}"),
            ));
        }
        let is_last = i + 1 == components.len();
        if is_last {
            match expect {
                PathKind::File if !meta.file_type().is_file() => {
                    return Err(DriverError::new(
                        DriverErrorKind::NotFound,
                        format!("not a regular file under release: {rel}"),
                    ));
                }
                PathKind::Dir if !meta.file_type().is_dir() => {
                    return Err(DriverError::new(
                        DriverErrorKind::Prepare,
                        format!("workdir is not a directory: {rel}"),
                    ));
                }
                _ => {}
            }
        } else if !meta.file_type().is_dir() {
            return Err(DriverError::new(
                DriverErrorKind::Policy,
                format!("non-directory intermediate in {rel:?}"),
            ));
        }
        cur = next;
    }
    Ok(cur)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PathKind {
    File,
    Dir,
}

/// Ensure `path` has no `..` components and is not itself a symlink before
/// destructive cleanup (GUMP-N001).
pub(crate) fn assert_owned_cleanup_target(path: &Path) -> Result<(), DriverError> {
    for c in path.components() {
        if matches!(c, Component::ParentDir) {
            return Err(DriverError::new(
                DriverErrorKind::Policy,
                format!("cleanup refuses parent traversal: {}", path.display()),
            ));
        }
    }
    if !path.exists() {
        return Ok(());
    }
    let meta = fs::symlink_metadata(path).map_err(|e| {
        DriverError::new(
            DriverErrorKind::Io,
            format!("stat cleanup target {}: {e}", path.display()),
        )
    })?;
    if meta.file_type().is_symlink() {
        return Err(DriverError::new(
            DriverErrorKind::Policy,
            format!("cleanup refuses symlink attempt root: {}", path.display()),
        ));
    }
    if !meta.file_type().is_dir() {
        return Err(DriverError::new(
            DriverErrorKind::Cleanup,
            format!("cleanup target is not a directory: {}", path.display()),
        ));
    }
    Ok(())
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
        let dir = std::env::temp_dir().join(format!("gump-n001-{name}-{nanos}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn rejects_parent_and_absolute() {
        assert!(rel_components("../etc/passwd").is_err());
        assert!(rel_components("/bin/sh").is_err());
        assert!(rel_components("a/../../b").is_err());
    }

    #[test]
    fn rejects_symlink_escape() {
        let root = tmp("sym");
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::write(root.join("bin/real"), b"ok").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/passwd", root.join("bin/evil")).unwrap();
            let err = resolve_beneath(&root, "bin/evil", PathKind::File).unwrap_err();
            assert_eq!(err.kind(), DriverErrorKind::Policy);
            std::os::unix::fs::symlink("/tmp", root.join("wd")).unwrap();
            let err = resolve_beneath(&root, "wd", PathKind::Dir).unwrap_err();
            assert_eq!(err.kind(), DriverErrorKind::Policy);
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_refuses_symlink_root() {
        let base = tmp("clean");
        let outside = base.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret"), b"x").unwrap();
        let link = base.join("attempt");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            assert!(assert_owned_cleanup_target(&link).is_err());
            assert!(outside.join("secret").is_file());
        }
        let _ = fs::remove_dir_all(base);
    }
}
