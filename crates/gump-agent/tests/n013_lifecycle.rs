//! GUMP-N013 / R09: lifecycle checks, retry/backoff, terminal reasons.
//!
//! Authority: docs/v1/NEXT_ACTIONS.md GUMP-N013, RUNTIME.md §9 / §11.

use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use gump_agent::{
    AcceptedPlacement, AttemptPhase, AuthorityState, CheckKind, CheckSpec, EffectExecutor,
    LifecycleContract, RetryPolicy, reasons,
};
use gump_driver::{DriverKind, NativeDriver, RuntimeSpec};
use gump_types::{AttemptId, UnitId};

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

fn release_with_bin(dir: &std::path::Path, script: &str) -> (PathBuf, RuntimeSpec) {
    let bin = dir.join("bin").join("work.sh");
    write_executable(&bin, &format!("#!/bin/sh\n{script}\n"));
    let runtime = RuntimeSpec {
        kind: DriverKind::Native,
        command: vec!["bin/work.sh".into()],
        interpreter: None,
        workdir: None,
    };
    (dir.to_path_buf(), runtime)
}

fn placement(
    release: PathBuf,
    runtime: RuntimeSpec,
    fence: u64,
    finite: bool,
    lifecycle: LifecycleContract,
) -> AcceptedPlacement {
    AcceptedPlacement {
        attempt_id: AttemptId::new(),
        unit_id: UnitId::new(),
        placement_fence: fence,
        release_root: release,
        runtime,
        lifecycle_finite: finite,
        capsule_verified: true,
        lifecycle,
        hiccup: None,
    }
}

#[test]
fn no_checks_never_infers_readiness_or_publication() {
    let tmp = tempfile::tempdir().unwrap();
    let release_dir = tmp.path().join("release");
    fs::create_dir_all(&release_dir).unwrap();
    let (release, runtime) = release_with_bin(&release_dir, "sleep 2");
    let p = placement(release, runtime, 1, false, LifecycleContract::default());
    let id = p.attempt_id;

    let mut exec = EffectExecutor::new(
        NativeDriver::new(),
        tmp.path().join("attempts"),
        AuthorityState::connected(1, 1),
    );
    let desired = vec![p];
    let reports = exec.reconcile(&desired, 0).unwrap();
    let r = reports.iter().find(|r| r.attempt_id == id).unwrap();
    assert!(matches!(r.phase, AttemptPhase::Running));
    assert_eq!(r.ready, None, "readiness must not be inferred");
    assert_eq!(
        r.publication_eligible, None,
        "publication must not be inferred"
    );
    exec.cancel(id).unwrap();
}

#[test]
fn http_readiness_sets_ready_and_optional_publication() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            use std::io::{Read, Write};
            let mut buf = [0u8; 256];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        }
    });

    let tmp = tempfile::tempdir().unwrap();
    let release_dir = tmp.path().join("release");
    fs::create_dir_all(&release_dir).unwrap();
    let (release, runtime) = release_with_bin(&release_dir, "sleep 5");
    let lifecycle = LifecycleContract {
        readiness: Some(CheckSpec {
            kind: CheckKind::Http,
            target: Some(format!("http://{addr}/health")),
            command: None,
            interval_ms: 1,
            timeout_ms: 500,
            initial_delay_ms: 0,
            success_threshold: 1,
            failure_threshold: 1,
            max_output_bytes: 4096,
        }),
        declares_publication: true,
        ..LifecycleContract::default()
    };
    let p = placement(release, runtime, 2, false, lifecycle);
    let id = p.attempt_id;

    let mut exec = EffectExecutor::new(
        NativeDriver::new(),
        tmp.path().join("attempts"),
        AuthorityState::connected(1, 2),
    )
    .with_check_budget_ms(500);
    let desired = vec![p];
    let mut ready = false;
    for tick in 0..40u64 {
        let reports = exec.reconcile(&desired, tick * 50).unwrap();
        if let Some(r) = reports.iter().find(|r| r.attempt_id == id) {
            if r.ready == Some(true) && r.publication_eligible == Some(true) {
                ready = true;
                break;
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert!(ready, "HTTP readiness should pass");
    exec.cancel(id).unwrap();
}

#[test]
fn command_check_marks_readiness() {
    let tmp = tempfile::tempdir().unwrap();
    let release_dir = tmp.path().join("release");
    fs::create_dir_all(&release_dir).unwrap();
    let probe = tmp.path().join("probe.sh");
    write_executable(&probe, "#!/bin/sh\nexit 0\n");
    let (release, runtime) = release_with_bin(&release_dir, "sleep 3");
    let lifecycle = LifecycleContract {
        readiness: Some(CheckSpec {
            kind: CheckKind::Command,
            target: None,
            command: Some(vec![probe.to_string_lossy().into_owned()]),
            interval_ms: 1,
            timeout_ms: 500,
            initial_delay_ms: 0,
            success_threshold: 1,
            failure_threshold: 1,
            max_output_bytes: 4096,
        }),
        ..LifecycleContract::default()
    };
    let p = placement(release, runtime, 3, false, lifecycle);
    let id = p.attempt_id;

    let mut exec = EffectExecutor::new(
        NativeDriver::new(),
        tmp.path().join("attempts"),
        AuthorityState::connected(1, 3),
    )
    .with_check_budget_ms(500);
    let desired = vec![p];
    let mut ready = false;
    for tick in 0..30u64 {
        let reports = exec.reconcile(&desired, tick * 20).unwrap();
        if reports
            .iter()
            .any(|r| r.attempt_id == id && r.ready == Some(true))
        {
            ready = true;
            break;
        }
        thread::sleep(Duration::from_millis(15));
    }
    assert!(ready);
    assert_eq!(
        exec.report(id).unwrap().publication_eligible,
        None,
        "publication remains undeclared"
    );
    exec.cancel(id).unwrap();
}

#[test]
fn finite_completion_cleans_and_explains() {
    let tmp = tempfile::tempdir().unwrap();
    let release_dir = tmp.path().join("release");
    fs::create_dir_all(&release_dir).unwrap();
    let (release, runtime) = release_with_bin(&release_dir, "exit 0");
    let p = placement(release, runtime, 4, true, LifecycleContract::default());
    let id = p.attempt_id;

    let mut exec = EffectExecutor::new(
        NativeDriver::new(),
        tmp.path().join("attempts"),
        AuthorityState::connected(1, 4),
    );
    let desired = vec![p];
    exec.reconcile(&desired, 0).unwrap();
    for _ in 0..80 {
        let reports = exec.reconcile(&desired, 0).unwrap();
        if reports.is_empty() && exec.live_count() == 0 {
            break;
        }
        // Capture terminal reason before cleanup if still present.
        if let Some(r) = reports.iter().find(|r| r.attempt_id == id) {
            if matches!(r.phase, AttemptPhase::Terminal { .. }) {
                assert_eq!(
                    r.terminal_reason.as_ref().map(|t| t.code),
                    Some(reasons::COMPLETED)
                );
            }
        }
        thread::sleep(Duration::from_millis(15));
    }
    assert_eq!(exec.live_count(), 0);
    assert!(!exec.attempt_root_exists(id));
}

#[test]
fn continuous_restart_schedules_backoff_then_runs_again() {
    let tmp = tempfile::tempdir().unwrap();
    let release_dir = tmp.path().join("release");
    fs::create_dir_all(&release_dir).unwrap();
    // Fail once then succeed by checking a marker file created after first exit.
    let marker = tmp.path().join("attempt.count");
    let script = format!(
        r#"
c=0
if [ -f "{m}" ]; then c=$(cat "{m}"); fi
c=$((c+1))
echo "$c" > "{m}"
if [ "$c" -eq 1 ]; then exit 7; fi
sleep 5
"#,
        m = marker.display()
    );
    let (release, runtime) = release_with_bin(&release_dir, &script);
    let lifecycle = LifecycleContract {
        retry: RetryPolicy {
            max_attempts: 3,
            initial_backoff_ms: 10,
            max_backoff_ms: 50,
            jitter_pct: 0,
            reset_window_ms: 60_000,
        },
        ..LifecycleContract::default()
    };
    let p = placement(release, runtime, 5, false, lifecycle);
    let id = p.attempt_id;

    let mut exec = EffectExecutor::new(
        NativeDriver::new(),
        tmp.path().join("attempts"),
        AuthorityState::connected(1, 5),
    );
    let desired = vec![p];
    exec.reconcile(&desired, 0).unwrap();

    let mut saw_retry = false;
    let mut saw_second = false;
    for tick in 0..120u64 {
        let now = tick * 25;
        let reports = exec.reconcile(&desired, now).unwrap();
        if let Some(r) = reports.iter().find(|r| r.attempt_id == id) {
            if matches!(r.phase, AttemptPhase::AwaitingRestart { .. }) {
                assert_eq!(
                    r.terminal_reason.as_ref().map(|t| t.code),
                    Some(reasons::RETRY_SCHEDULED)
                );
                saw_retry = true;
            }
            if r.attempt_index >= 2 && matches!(r.phase, AttemptPhase::Running) {
                saw_second = true;
                break;
            }
        }
        thread::sleep(Duration::from_millis(15));
    }
    assert!(saw_retry, "expected retry schedule");
    assert!(saw_second, "expected restarted attempt");
    exec.cancel(id).unwrap();
}

#[test]
fn permanent_failure_when_retries_exhausted() {
    let tmp = tempfile::tempdir().unwrap();
    let release_dir = tmp.path().join("release");
    fs::create_dir_all(&release_dir).unwrap();
    let (release, runtime) = release_with_bin(&release_dir, "exit 9");
    let lifecycle = LifecycleContract {
        retry: RetryPolicy {
            max_attempts: 2,
            initial_backoff_ms: 5,
            max_backoff_ms: 20,
            jitter_pct: 0,
            reset_window_ms: 60_000,
        },
        ..LifecycleContract::default()
    };
    let p = placement(release, runtime, 6, true, lifecycle);
    let id = p.attempt_id;

    let mut exec = EffectExecutor::new(
        NativeDriver::new(),
        tmp.path().join("attempts"),
        AuthorityState::connected(1, 6),
    );
    let desired = vec![p];
    let mut permanent = false;
    for tick in 0..100u64 {
        let reports = exec.reconcile(&desired, tick * 30).unwrap();
        if let Some(r) = reports.iter().find(|r| r.attempt_id == id) {
            if matches!(r.phase, AttemptPhase::PermanentFailure) {
                assert_eq!(
                    r.terminal_reason.as_ref().map(|t| t.code),
                    Some(reasons::PERMANENT_FAILURE)
                );
                permanent = true;
                break;
            }
        }
        thread::sleep(Duration::from_millis(15));
    }
    assert!(permanent);
}

#[test]
fn check_budget_does_not_block_reconcile() {
    let tmp = tempfile::tempdir().unwrap();
    let release_dir = tmp.path().join("release");
    fs::create_dir_all(&release_dir).unwrap();
    let (release, runtime) = release_with_bin(&release_dir, "sleep 2");
    // Point HTTP at a black-hole port with a long timeout; budget must skip.
    let lifecycle = LifecycleContract {
        readiness: Some(CheckSpec {
            kind: CheckKind::Http,
            target: Some("http://127.0.0.1:1/".into()),
            command: None,
            interval_ms: 1,
            timeout_ms: 5_000,
            initial_delay_ms: 0,
            success_threshold: 1,
            failure_threshold: 1,
            max_output_bytes: 4096,
        }),
        ..LifecycleContract::default()
    };
    let p = placement(release, runtime, 7, false, lifecycle);
    let id = p.attempt_id;

    let mut exec = EffectExecutor::new(
        NativeDriver::new(),
        tmp.path().join("attempts"),
        AuthorityState::connected(1, 7),
    )
    .with_check_budget_ms(5);
    let desired = vec![p];
    let start = std::time::Instant::now();
    let _ = exec.reconcile(&desired, 0).unwrap();
    let _ = exec.reconcile(&desired, 50).unwrap();
    assert!(
        start.elapsed() < Duration::from_millis(800),
        "reconcile must not wait on full check timeout"
    );
    // Still undeclared publication; readiness may be Some(false) after failed/skipped.
    let r = exec.report(id).unwrap();
    assert_eq!(r.publication_eligible, None);
    exec.cancel(id).unwrap();
}

#[test]
fn backoff_is_bounded_and_deterministic_without_jitter() {
    let policy = RetryPolicy {
        max_attempts: 8,
        initial_backoff_ms: 1_000,
        max_backoff_ms: 5_000,
        jitter_pct: 0,
        reset_window_ms: 60_000,
    };
    assert_eq!(policy.backoff_ms(1, 0), 1_000);
    assert_eq!(policy.backoff_ms(2, 0), 2_000);
    assert_eq!(policy.backoff_ms(3, 0), 4_000);
    assert_eq!(policy.backoff_ms(4, 0), 5_000);
    assert_eq!(policy.backoff_ms(8, 99), 5_000);
}
