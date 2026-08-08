//! F02 exit evidence: race, escape, and sensitive-file capture tests.
//!
//! Authority: docs/v1/DELIVERY.md F02, docs/v1/FORMATS.md §11, DECISIONS D009.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gump_manifest::capture::{
    CaptureErrorKind, CapturePlan, apply_prepare_outputs, capture_workspace, verify_captured_bytes,
};
use gump_manifest::{Package, PackageFormat, PrepareOutput};

fn tmp_workspace(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("gump-f02-{name}-{nanos}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn package(include: &[&str], exclude: &[&str], allow_root: bool, allow_sensitive: bool) -> Package {
    Package {
        root: ".".into(),
        include: include.iter().map(|s| (*s).to_string()).collect(),
        exclude: exclude.iter().map(|s| (*s).to_string()).collect(),
        format: PackageFormat::TarZstd,
        allow_workspace_root: allow_root,
        allow_sensitive_files: allow_sensitive,
    }
}

fn write(root: &Path, rel: &str, body: &[u8]) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

#[test]
fn captures_allowlisted_files_only() {
    let root = tmp_workspace("allow");
    write(&root, "bin/hello", b"hi");
    write(&root, "bin/skipme", b"no");
    write(&root, "README.md", b"doc");
    let plan = CapturePlan::from_package(&package(&["bin/hello"], &[], false, false)).unwrap();
    let tree = capture_workspace(&root, &plan).unwrap();
    assert_eq!(tree.len(), 1);
    assert!(tree.get("bin/hello").is_some());
    assert!(tree.get("bin/skipme").is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn rejects_path_escape_patterns_in_plan_targets() {
    // Escape via prepare `to` path.
    let root = tmp_workspace("escape");
    let staging = root.join("staging");
    fs::create_dir_all(&staging).unwrap();
    fs::write(staging.join("out.bin"), b"x").unwrap();
    let mut tree = Default::default();
    let err = apply_prepare_outputs(
        &root,
        &mut tree,
        &staging,
        &[PrepareOutput {
            from: "out.bin".into(),
            to: "../outside".into(),
        }],
        false,
    )
    .unwrap_err();
    assert_eq!(err.kind(), CaptureErrorKind::Escape);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn denies_sensitive_files_unless_explicitly_allowed() {
    let root = tmp_workspace("sensitive");
    write(&root, "bin/hello", b"ok");
    write(&root, ".env", b"SECRET=1");
    write(&root, "secrets/id_rsa", b"key");
    let plan = CapturePlan::from_package(&package(&["**"], &[], true, false)).unwrap();
    let err = capture_workspace(&root, &plan).unwrap_err();
    assert_eq!(err.kind(), CaptureErrorKind::Sensitive);

    let plan_ok = CapturePlan::from_package(&package(&["bin/hello"], &[], false, false)).unwrap();
    let tree = capture_workspace(&root, &plan_ok).unwrap();
    assert_eq!(tree.len(), 1);

    // Explicit allow still captures only matched includes.
    write(&root, "bin/.env", b"x");
    let plan_allow = CapturePlan::from_package(&package(&["bin/**"], &[], false, true)).unwrap();
    let tree = capture_workspace(&root, &plan_allow).unwrap();
    assert!(tree.get("bin/.env").is_some());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn workspace_root_include_requires_ack() {
    let err = CapturePlan::from_package(&package(&["."], &[], false, false)).unwrap_err();
    assert_eq!(err.kind(), CaptureErrorKind::Policy);
    assert!(CapturePlan::from_package(&package(&["."], &[], true, false)).is_ok());
}

#[test]
fn prepare_outputs_enter_virtual_tree() {
    let root = tmp_workspace("prepare");
    write(&root, "src/a.rs", b"fn main(){}");
    let staging = root.join(".gump-staging"); // staging name is fine; not captured
    fs::create_dir_all(&staging).unwrap();
    fs::write(staging.join("hello"), b"binary").unwrap();

    let plan = CapturePlan::from_package(&package(&["src/**"], &[], false, false)).unwrap();
    let mut tree = capture_workspace(&root, &plan).unwrap();
    apply_prepare_outputs(
        &root,
        &mut tree,
        &staging,
        &[PrepareOutput {
            from: "hello".into(),
            to: "bin/hello".into(),
        }],
        false,
    )
    .unwrap();
    assert!(tree.get("src/a.rs").is_some());
    let prepared = tree.get("bin/hello").unwrap();
    assert!(prepared.from_prepare);
    assert_eq!(prepared.identity.len, 6);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn source_changed_when_file_mutates_between_passes() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    let root = tmp_workspace("race");
    write(&root, "bin/hello", b"v1");
    let plan = CapturePlan::from_package(&package(&["bin/hello"], &[], false, false)).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let path = root.join("bin/hello");
    let stop_w = Arc::clone(&stop);
    let writer = thread::spawn(move || {
        let mut n = 0u64;
        while !stop_w.load(Ordering::Relaxed) {
            let _ = fs::write(&path, format!("v{n}").as_bytes());
            n = n.wrapping_add(1);
            thread::yield_now();
        }
    });

    let mut saw_source_changed = false;
    for _ in 0..200 {
        match capture_workspace(&root, &plan) {
            Err(e) if e.kind() == CaptureErrorKind::SourceChanged => {
                saw_source_changed = true;
                break;
            }
            Ok(_) | Err(_) => thread::sleep(Duration::from_micros(50)),
        }
    }
    stop.store(true, Ordering::Relaxed);
    let _ = writer.join();
    assert!(
        saw_source_changed,
        "expected SOURCE_CHANGED under concurrent mutation"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn rejects_symlink_as_escape() {
    let root = tmp_workspace("symlink");
    write(&root, "bin/hello", b"ok");
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let _ = symlink("/etc/passwd", root.join("bin/link"));
        let plan = CapturePlan::from_package(&package(&["bin/**"], &[], false, false)).unwrap();
        let err = capture_workspace(&root, &plan).unwrap_err();
        assert_eq!(err.kind(), CaptureErrorKind::Escape);
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn rejects_intermediate_directory_symlink_at_open() {
    // STL-16: O_NOFOLLOW on the leaf alone is insufficient — an intermediate
    // directory symlink must fail closed under the root-handle open.
    let root = tmp_workspace("mid-dir-symlink");
    fs::create_dir_all(root.join("via")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink("/etc", root.join("via/mid")).unwrap();
        // Direct open path (bypasses read_dir discovery) must not follow mid.
        // Capture walk also rejects when it encounters the symlink entry.
        let plan = CapturePlan::from_package(&package(&["via/**"], &[], false, false)).unwrap();
        let err = capture_workspace(&root, &plan).unwrap_err();
        assert_eq!(err.kind(), CaptureErrorKind::Escape);
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn retained_bytes_survive_symlink_swap_after_capture() {
    // STL-05: after capture, replacing the path with a symlink must not change
    // archived bytes (pack uses retained content, not a fresh follow-open).
    let root = tmp_workspace("toctou-symlink");
    write(&root, "bin/hello", b"trusted-payload");
    let plan = CapturePlan::from_package(&package(&["bin/hello"], &[], false, false)).unwrap();
    let tree = capture_workspace(&root, &plan).unwrap();
    let entry = tree.get("bin/hello").unwrap().clone();
    assert_eq!(entry.bytes, b"trusted-payload");
    verify_captured_bytes(&entry).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        fs::remove_file(root.join("bin/hello")).unwrap();
        symlink("/etc/passwd", root.join("bin/hello")).unwrap();
        // Workspace path now points at a host file if followed.
        let followed = fs::read(root.join("bin/hello")).unwrap();
        assert_ne!(followed, b"trusted-payload");
        // Retained capture bytes stay trusted.
        assert_eq!(entry.bytes, b"trusted-payload");
        verify_captured_bytes(&entry).unwrap();
        // No-follow open of the swapped path must fail closed.
        let err = capture_workspace(&root, &plan).unwrap_err();
        assert_eq!(err.kind(), CaptureErrorKind::Escape);
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn retained_bytes_survive_mutate_after_capture() {
    let root = tmp_workspace("toctou-mutate");
    write(&root, "bin/hello", b"v1");
    let plan = CapturePlan::from_package(&package(&["bin/hello"], &[], false, false)).unwrap();
    let tree = capture_workspace(&root, &plan).unwrap();
    let entry = tree.get("bin/hello").unwrap().clone();
    write(&root, "bin/hello", b"v2-mutated-after-capture");
    assert_eq!(entry.bytes, b"v1");
    verify_captured_bytes(&entry).unwrap();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn verify_captured_bytes_detects_tampered_buffer() {
    let root = tmp_workspace("tamper");
    write(&root, "bin/hello", b"ok");
    let plan = CapturePlan::from_package(&package(&["bin/hello"], &[], false, false)).unwrap();
    let tree = capture_workspace(&root, &plan).unwrap();
    let mut entry = tree.get("bin/hello").unwrap().clone();
    entry.bytes.push(b'!');
    let err = verify_captured_bytes(&entry).unwrap_err();
    assert_eq!(err.kind(), CaptureErrorKind::SourceChanged);
    let _ = fs::remove_dir_all(root);
}
