//! STL-04 evidence: pipe drains + TERM→KILL process-group supervision.
//!
//! Authority: docs/v1/RUNTIME.md §9 / §16, DELIVERY R06, stop-the-line STL-04.

#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gump_driver::{
    AttemptContext, CAPTURE_RING_BYTES, Driver, DriverKind, IoEndpoints, NativeDriver, ReleaseRoot,
    ResourceGrant, RuntimeSpec, SecretPlan, Signal, StartFence,
};
use gump_types::AttemptId;

/// Serialize process-group signal tests (parallel workers flake on pipe counts).
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tmp(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("gump-stl04-{name}-{nanos}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_executable(path: &std::path::Path, body: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn start_native(
    release: &ReleaseRoot,
    command: &str,
    capture: bool,
) -> (gump_driver::RunningHandle, PathBuf) {
    let driver = NativeDriver::new();
    let base = tmp("attempt");
    let attempt_root = base.join("attempt");
    fs::create_dir_all(&attempt_root).unwrap();
    let prepared = driver
        .prepare(
            release,
            &RuntimeSpec {
                kind: DriverKind::Native,
                command: vec![command.into()],
                interpreter: None,
                workdir: None,
            },
            &AttemptContext {
                attempt_id: AttemptId::new(),
                attempt_root: attempt_root.clone(),
            },
        )
        .unwrap();
    let admission = driver
        .admit(
            prepared,
            ResourceGrant {
                max_processes: Some(16),
            },
            &SecretPlan { deferred: true },
        )
        .unwrap();
    let running = driver
        .start(
            admission,
            StartFence { generation: 1 },
            &IoEndpoints {
                capture_stdout: capture,
                capture_stderr: capture,
            },
        )
        .unwrap();
    (running, attempt_root)
}

#[test]
fn infinite_stdout_does_not_hang_when_drained() {
    let _guard = test_lock();
    let root = tmp("flood");
    write_executable(&root.join("bin/flood"), "#!/bin/sh\nexec yes\n");
    let release = ReleaseRoot::new(&root);
    let driver = NativeDriver::new();
    let (mut running, _) = start_native(&release, "bin/flood", true);

    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(500) {
        assert!(driver.observe(&mut running).unwrap().running);
        if running.stdout_received_bytes() > 64 * 1024 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        running.stdout_received_bytes() > 64 * 1024,
        "drain should have pulled pipe bytes, got {}",
        running.stdout_received_bytes()
    );
    driver
        .terminate(&mut running, Duration::from_millis(300))
        .unwrap();
    assert!(!driver.observe(&mut running).unwrap().running);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn term_then_kill_reaps_ignore_term_child() {
    let _guard = test_lock();
    let root = tmp("ignore-term");
    write_executable(
        &root.join("bin/stubborn"),
        concat!(
            "#!/bin/sh\n",
            "trap '' TERM\n",
            "printf x >\"$GUMP_ATTEMPT_ROOT/ready\"\n",
            "while true; do sleep 1; done\n",
        ),
    );
    let release = ReleaseRoot::new(&root);
    let driver = NativeDriver::new();
    let (mut running, attempt_root) = start_native(&release, "bin/stubborn", false);
    // Wait until trap is armed (ready file written after `trap`).
    let ready = attempt_root.join("ready");
    let armed = Instant::now();
    while !ready.is_file() && armed.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(ready.is_file(), "stubborn never armed TERM trap");
    assert!(
        driver.observe(&mut running).unwrap().running,
        "stubborn child must stay up before TERM"
    );

    driver.signal(&mut running, Signal::Term).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        driver.observe(&mut running).unwrap().running,
        "SIGTERM ignored — still running"
    );

    driver
        .terminate(&mut running, Duration::from_millis(200))
        .unwrap();
    assert!(
        !driver.observe(&mut running).unwrap().running,
        "SIGKILL after deadline must reap"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn process_group_kill_reaps_grandchild() {
    let _guard = test_lock();
    let root = tmp("grand");
    write_executable(
        &root.join("bin/parent"),
        concat!("#!/bin/sh\n", "sleep 30 &\n", "wait\n"),
    );
    let release = ReleaseRoot::new(&root);
    let driver = NativeDriver::new();
    let (mut running, _) = start_native(&release, "bin/parent", false);
    std::thread::sleep(Duration::from_millis(50));
    driver.kill(&mut running).unwrap();
    assert!(!driver.observe(&mut running).unwrap().running);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn capture_ring_drops_oldest_under_pressure() {
    let _guard = test_lock();
    let root = tmp("ring");
    write_executable(
        &root.join("bin/big"),
        &format!(
            "#!/bin/sh\ndd if=/dev/zero bs={} count=3 2>/dev/null\n",
            CAPTURE_RING_BYTES
        ),
    );
    let release = ReleaseRoot::new(&root);
    let driver = NativeDriver::new();
    let (mut running, _) = start_native(&release, "bin/big", true);
    for _ in 0..500 {
        if !driver.observe(&mut running).unwrap().running {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        running.stdout_received_bytes() as usize >= CAPTURE_RING_BYTES,
        "should have received at least one ring's worth"
    );
    assert!(
        running.captured_stdout().len() <= CAPTURE_RING_BYTES,
        "ring must stay bounded"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn infinite_stderr_does_not_hang_when_drained() {
    let _guard = test_lock();
    let root = tmp("flood-err");
    write_executable(&root.join("bin/flooderr"), "#!/bin/sh\nexec yes 1>&2\n");
    let release = ReleaseRoot::new(&root);
    let driver = NativeDriver::new();
    let (mut running, _) = start_native(&release, "bin/flooderr", true);

    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(500) {
        assert!(driver.observe(&mut running).unwrap().running);
        if running.stderr_received_bytes() > 64 * 1024 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        running.stderr_received_bytes() > 64 * 1024,
        "stderr drain should have pulled pipe bytes, got {}",
        running.stderr_received_bytes()
    );
    driver
        .terminate(&mut running, Duration::from_millis(300))
        .unwrap();
    assert!(!driver.observe(&mut running).unwrap().running);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn binary_stdout_drain_preserves_nul_bytes() {
    let _guard = test_lock();
    let root = tmp("binary");
    write_executable(&root.join("bin/binout"), "#!/bin/sh\nprintf 'a\\0b\\0c'\n");
    let release = ReleaseRoot::new(&root);
    let driver = NativeDriver::new();
    let (mut running, _) = start_native(&release, "bin/binout", true);
    for _ in 0..200 {
        if !driver.observe(&mut running).unwrap().running {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let out = running.captured_stdout();
    assert!(
        out.contains(&0u8),
        "drain must keep embedded NUL bytes, got {out:?}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn fork_and_exit_parent_then_kill_tree() {
    let _guard = test_lock();
    let root = tmp("forkexit");
    // Parent exits immediately; background sleep remains in the process group.
    write_executable(
        &root.join("bin/forkexit"),
        "#!/bin/sh\nsleep 30 &\nexit 0\n",
    );
    let release = ReleaseRoot::new(&root);
    let driver = NativeDriver::new();
    let (mut running, _) = start_native(&release, "bin/forkexit", false);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        if !driver.observe(&mut running).unwrap().running {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    driver.kill(&mut running).unwrap();
    assert!(!driver.observe(&mut running).unwrap().running);
    let _ = fs::remove_dir_all(root);
}
