//! GUMP-N001: runtime paths stay beneath the verified release / owned attempt root.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use gump_driver::{
    AttemptContext, Driver, DriverErrorKind, DriverKind, NativeDriver, ReleaseRoot, RuntimeSpec,
    ScriptDriver,
};
use gump_types::AttemptId;

fn tmp(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("gump-n001-it-{name}-{nanos}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_executable(path: &std::path::Path, body: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn attempt_ctx(base: &std::path::Path) -> AttemptContext {
    let attempt_root = base.join("attempt");
    AttemptContext {
        attempt_id: AttemptId::new(),
        attempt_root,
    }
}

#[test]
fn native_rejects_parent_traversal_in_command() {
    let root = tmp("dotdot-cmd");
    write_executable(&root.join("bin/hello"), "#!/bin/sh\nexit 0\n");
    let release = ReleaseRoot::new(&root);
    let err = NativeDriver::new()
        .prepare(
            &release,
            &RuntimeSpec {
                kind: DriverKind::Native,
                command: vec!["../bin/hello".into()],
                interpreter: None,
                workdir: None,
            },
            &attempt_ctx(&root),
        )
        .unwrap_err();
    assert_eq!(err.kind(), DriverErrorKind::Policy);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_rejects_absolute_command() {
    let root = tmp("abs-cmd");
    write_executable(&root.join("bin/hello"), "#!/bin/sh\nexit 0\n");
    let release = ReleaseRoot::new(&root);
    let err = NativeDriver::new()
        .prepare(
            &release,
            &RuntimeSpec {
                kind: DriverKind::Native,
                command: vec!["/bin/sh".into()],
                interpreter: None,
                workdir: None,
            },
            &attempt_ctx(&root),
        )
        .unwrap_err();
    assert_eq!(err.kind(), DriverErrorKind::Policy);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_rejects_symlink_command() {
    let root = tmp("sym-cmd");
    write_executable(&root.join("bin/real"), "#!/bin/sh\nexit 0\n");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/bin/sh", root.join("bin/evil")).unwrap();
        let release = ReleaseRoot::new(&root);
        let err = NativeDriver::new()
            .prepare(
                &release,
                &RuntimeSpec {
                    kind: DriverKind::Native,
                    command: vec!["bin/evil".into()],
                    interpreter: None,
                    workdir: None,
                },
                &attempt_ctx(&root),
            )
            .unwrap_err();
        assert_eq!(err.kind(), DriverErrorKind::Policy);
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn workdir_rejects_symlink_escape() {
    let root = tmp("sym-wd");
    write_executable(&root.join("bin/hello"), "#!/bin/sh\nexit 0\n");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/tmp", root.join("out")).unwrap();
        let release = ReleaseRoot::new(&root);
        let err = NativeDriver::new()
            .prepare(
                &release,
                &RuntimeSpec {
                    kind: DriverKind::Native,
                    command: vec!["bin/hello".into()],
                    interpreter: None,
                    workdir: Some("out".into()),
                },
                &attempt_ctx(&root),
            )
            .unwrap_err();
        assert_eq!(err.kind(), DriverErrorKind::Policy);
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn workdir_rejects_parent_traversal() {
    let root = tmp("dotdot-wd");
    write_executable(&root.join("bin/hello"), "#!/bin/sh\nexit 0\n");
    let release = ReleaseRoot::new(&root);
    let err = NativeDriver::new()
        .prepare(
            &release,
            &RuntimeSpec {
                kind: DriverKind::Native,
                command: vec!["bin/hello".into()],
                interpreter: None,
                workdir: Some("..".into()),
            },
            &attempt_ctx(&root),
        )
        .unwrap_err();
    assert_eq!(err.kind(), DriverErrorKind::Policy);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cleanup_refuses_symlink_attempt_root() {
    let root = tmp("clean-sym");
    write_executable(&root.join("bin/hello"), "#!/bin/sh\nexit 0\n");
    let outside = root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("keep"), b"secret").unwrap();
    let attempt_link = root.join("attempt-link");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, &attempt_link).unwrap();
        let release = ReleaseRoot::new(&root);
        let driver = NativeDriver::new();
        let err = driver
            .prepare(
                &release,
                &RuntimeSpec {
                    kind: DriverKind::Native,
                    command: vec!["bin/hello".into()],
                    interpreter: None,
                    workdir: None,
                },
                &AttemptContext {
                    attempt_id: AttemptId::new(),
                    attempt_root: attempt_link.clone(),
                },
            )
            .unwrap_err();
        assert_eq!(err.kind(), DriverErrorKind::Policy);
        // Outside must remain intact (cleanup/prepare must not follow the link).
        assert_eq!(fs::read(outside.join("keep")).unwrap(), b"secret");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn script_rejects_symlink_script_path() {
    let root = tmp("sym-script");
    fs::write(root.join("ok.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/passwd", root.join("evil.sh")).unwrap();
        let release = ReleaseRoot::new(&root);
        let err = ScriptDriver::new()
            .prepare(
                &release,
                &RuntimeSpec {
                    kind: DriverKind::Script,
                    command: vec!["evil.sh".into()],
                    interpreter: Some(vec!["/bin/sh".into()]),
                    workdir: None,
                },
                &attempt_ctx(&root),
            )
            .unwrap_err();
        assert_eq!(err.kind(), DriverErrorKind::Policy);
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn race_replace_command_with_symlink_fails_at_start() {
    // Narrow prepare→start TOCTOU: after prepare pins an absolute path, replacing
    // that path with a symlink must fail closed at start.
    let root = tmp("race-cmd");
    write_executable(&root.join("bin/hello"), "#!/bin/sh\nexit 0\n");
    let release = ReleaseRoot::new(&root);
    let driver = NativeDriver::new();
    let ctx = attempt_ctx(&root);
    let prepared = driver
        .prepare(
            &release,
            &RuntimeSpec {
                kind: DriverKind::Native,
                command: vec!["bin/hello".into()],
                interpreter: None,
                workdir: None,
            },
            &ctx,
        )
        .unwrap();
    #[cfg(unix)]
    {
        let pinned = root.join("bin/hello");
        fs::remove_file(&pinned).unwrap();
        std::os::unix::fs::symlink("/bin/sh", &pinned).unwrap();
        let admission = driver
            .admit(
                prepared,
                gump_driver::ResourceGrant {
                    max_processes: None,
                },
                &gump_driver::SecretPlan { deferred: true },
            )
            .unwrap();
        let err = driver
            .start(
                admission,
                gump_driver::StartFence { generation: 1 },
                &gump_driver::IoEndpoints::default(),
            )
            .unwrap_err();
        assert_eq!(err.kind(), DriverErrorKind::Policy);
    }
    #[cfg(not(unix))]
    {
        let _ = prepared;
    }
    let _ = fs::remove_dir_all(root);
}
