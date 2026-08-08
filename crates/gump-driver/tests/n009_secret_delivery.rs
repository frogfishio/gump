//! GUMP-N009 / S07: scoped env + FD injection; wrong fence rejected; no root leak.

use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gump_driver::{
    AttemptContext, DeliveryScope, Driver, DriverKind, InjectForm, IoEndpoints, NativeDriver,
    ReleaseRoot, ResourceGrant, RuntimeSpec, SecretPlan, SecretValue, StartFence,
};
use gump_types::{AttemptId, CapsuleId, ClusterId, Secret, WorkloadId};

fn tmp(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("gump-n009-{name}-{nanos}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn fixed_v7(tag: u8) -> [u8; 16] {
    let mut b = [
        0x01, 0x8f, 0x4a, 0x00, 0x00, 0x00, 0x70, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ];
    b[15] = tag;
    b
}

fn scope_for(attempt_id: AttemptId, fence: u64) -> DeliveryScope {
    DeliveryScope {
        cluster_id: ClusterId::from_bytes(fixed_v7(1)).unwrap(),
        workload_id: WorkloadId::from_bytes(fixed_v7(2)).unwrap(),
        release_id: CapsuleId::from_bytes(fixed_v7(3)).unwrap(),
        unit: 0,
        attempt_id,
        node_id: 1,
        controller_epoch: 1,
        placement_fence: fence,
    }
}

#[test]
fn env_and_fd_injection_round_trip_and_no_root_leak() {
    let root = tmp("inject");
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    // Print env TOKEN and first line from fd 7.
    fs::write(
        bin.join("hello"),
        "#!/bin/sh\nprintf '%s|' \"$TOKEN\"\nhead -c 64 <&7\nprintf '|'\nexit 0\n",
    )
    .unwrap();
    fs::set_permissions(bin.join("hello"), fs::Permissions::from_mode(0o755)).unwrap();

    let attempt_root = root.join("attempt");
    fs::create_dir_all(&attempt_root).unwrap();
    let attempt_id = AttemptId::from_bytes(fixed_v7(4)).unwrap();
    let canary = "n009-driver-canary-SECRET";
    let plan = SecretPlan::scoped(
        scope_for(attempt_id, 42),
        vec![
            SecretValue {
                logical_name: "TOKEN".into(),
                form: InjectForm::Env,
                bytes: Secret::new(canary.as_bytes().to_vec()),
            },
            SecretValue {
                logical_name: "FILE_SECRET".into(),
                form: InjectForm::Fd {
                    fd: 7,
                    reference_env: Some("SECRET_FD".into()),
                },
                bytes: Secret::new(b"fd-payload".to_vec()),
            },
        ],
    );

    let driver = NativeDriver::new();
    let prepared = driver
        .prepare(
            &ReleaseRoot::new(&root),
            &RuntimeSpec {
                kind: DriverKind::Native,
                command: vec!["bin/hello".into()],
                interpreter: None,
                workdir: None,
            },
            &AttemptContext {
                attempt_id,
                attempt_root: attempt_root.clone(),
            },
        )
        .unwrap();
    let admission = driver
        .admit(
            prepared,
            ResourceGrant {
                max_processes: None,
            },
            plan,
        )
        .unwrap();
    let mut running = driver
        .start(
            admission,
            StartFence { generation: 42 },
            &IoEndpoints {
                capture_stdout: true,
                capture_stderr: true,
                pipe_sink: None,
            },
        )
        .unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let obs = loop {
        let o = driver.observe(&mut running).unwrap();
        if !o.running {
            break o;
        }
        if std::time::Instant::now() > deadline {
            panic!("child did not exit");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(obs.exit_code, Some(0));
    let stdout = String::from_utf8_lossy(&running.captured_stdout()).into_owned();
    assert!(stdout.contains(canary), "stdout={stdout:?}");
    assert!(stdout.contains("fd-payload"), "stdout={stdout:?}");

    // Canary must not appear under release or attempt roots.
    for walk in [&root, &attempt_root] {
        for entry in walkdir_files(walk) {
            let mut bytes = Vec::new();
            fs::File::open(&entry)
                .unwrap()
                .read_to_end(&mut bytes)
                .unwrap();
            // hello script intentionally does not embed canary; only runtime env.
            if entry.ends_with("hello") {
                continue;
            }
            assert!(
                !bytes.windows(canary.len()).any(|w| w == canary.as_bytes()),
                "canary leaked into {}",
                entry.display()
            );
        }
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn wrong_fence_rejected_at_start() {
    let root = tmp("fence");
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("hello"), "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(bin.join("hello"), fs::Permissions::from_mode(0o755)).unwrap();
    let attempt_root = root.join("attempt");
    fs::create_dir_all(&attempt_root).unwrap();
    let attempt_id = AttemptId::from_bytes(fixed_v7(5)).unwrap();
    let plan = SecretPlan::scoped(
        scope_for(attempt_id, 1),
        vec![SecretValue {
            logical_name: "TOKEN".into(),
            form: InjectForm::Env,
            bytes: Secret::new(b"x".to_vec()),
        }],
    );
    let driver = NativeDriver::new();
    let prepared = driver
        .prepare(
            &ReleaseRoot::new(&root),
            &RuntimeSpec {
                kind: DriverKind::Native,
                command: vec!["bin/hello".into()],
                interpreter: None,
                workdir: None,
            },
            &AttemptContext {
                attempt_id,
                attempt_root,
            },
        )
        .unwrap();
    let admission = driver
        .admit(
            prepared,
            ResourceGrant {
                max_processes: None,
            },
            plan,
        )
        .unwrap();
    let err = driver
        .start(
            admission,
            StartFence { generation: 99 },
            &IoEndpoints::default(),
        )
        .unwrap_err();
    assert!(err.message().contains("fence"));
    let _ = fs::remove_dir_all(root);
}

fn walkdir_files(root: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.is_file() {
                out.push(p);
            }
        }
    }
    walk(root, &mut out);
    out
}
