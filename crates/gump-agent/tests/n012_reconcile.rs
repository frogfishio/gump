//! GUMP-N012 / R06 / R10: agent reconcile and supervision loop.
//!
//! Authority: docs/v1/NEXT_ACTIONS.md GUMP-N012, RUNTIME.md §4 / §10.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use gump_agent::{
    AcceptedPlacement, AgentError, AttemptPhase, AuthorityState, EffectExecutor, EffectKind,
    FenceError, IsolationPolicy, allow_effect,
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
    verified: bool,
) -> AcceptedPlacement {
    AcceptedPlacement {
        attempt_id: AttemptId::new(),
        unit_id: UnitId::new(),
        placement_fence: fence,
        release_root: release,
        runtime,
        lifecycle_finite: finite,
        capsule_verified: verified,
    }
}

fn wait_gone(exec: &mut EffectExecutor<NativeDriver>, desired: &[AcceptedPlacement], rounds: u32) {
    for _ in 0..rounds {
        let reports = exec.reconcile(desired, 0).unwrap();
        if reports.is_empty() && exec.live_count() == 0 {
            return;
        }
        if reports
            .iter()
            .all(|r| !matches!(r.phase, AttemptPhase::Running | AttemptPhase::Starting))
            && exec.live_count() == 0
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn finite_native_reaches_terminal_and_cleans_up() {
    let tmp = tempfile::tempdir().unwrap();
    let release_dir = tmp.path().join("release");
    fs::create_dir_all(&release_dir).unwrap();
    let (release, runtime) = release_with_bin(&release_dir, "exit 0");
    let p = placement(release, runtime, 7, true, true);
    let id = p.attempt_id;

    let mut exec = EffectExecutor::new(
        NativeDriver::new(),
        tmp.path().join("attempts"),
        AuthorityState::connected(1, 7),
    );
    let desired = vec![p];
    exec.reconcile(&desired, 0).unwrap();
    assert_eq!(exec.live_count(), 1);

    wait_gone(&mut exec, &desired, 100);
    assert_eq!(exec.live_count(), 0);
    assert!(!exec.attempt_root_exists(id));
}

#[test]
fn continuous_stays_until_intent_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let release_dir = tmp.path().join("release");
    fs::create_dir_all(&release_dir).unwrap();
    let (release, runtime) = release_with_bin(&release_dir, "sleep 30");
    let p = placement(release, runtime, 3, false, true);
    let id = p.attempt_id;

    let mut exec = EffectExecutor::new(
        NativeDriver::new(),
        tmp.path().join("attempts"),
        AuthorityState::connected(1, 3),
    );
    let desired = vec![p];
    exec.reconcile(&desired, 0).unwrap();
    assert_eq!(exec.live_count(), 1);
    std::thread::sleep(Duration::from_millis(50));
    let reports = exec.reconcile(&desired, 0).unwrap();
    assert!(matches!(reports[0].phase, AttemptPhase::Running));
    assert!(exec.attempt_root_exists(id));

    // Intent change → stop + cleanup.
    exec.reconcile(&[], 0).unwrap();
    assert_eq!(exec.live_count(), 0);
    assert!(!exec.attempt_root_exists(id));
}

#[test]
fn cancel_and_forced_kill_leave_no_root() {
    let tmp = tempfile::tempdir().unwrap();
    let release_dir = tmp.path().join("release");
    fs::create_dir_all(&release_dir).unwrap();
    let (release, runtime) = release_with_bin(&release_dir, "sleep 30");
    let p = placement(release.clone(), runtime.clone(), 5, false, true);
    let id = p.attempt_id;

    let mut exec = EffectExecutor::new(
        NativeDriver::new(),
        tmp.path().join("attempts"),
        AuthorityState::connected(1, 5),
    );
    exec.reconcile(&[p], 0).unwrap();
    exec.cancel(id).unwrap();
    assert_eq!(exec.live_count(), 0);
    assert!(!exec.attempt_root_exists(id));

    let p2 = placement(release, runtime, 5, false, true);
    let id2 = p2.attempt_id;
    exec.reconcile(&[p2], 0).unwrap();
    exec.force_kill(id2).unwrap();
    assert_eq!(exec.live_count(), 0);
    assert!(!exec.attempt_root_exists(id2));
}

#[test]
fn unverified_capsule_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let release_dir = tmp.path().join("release");
    fs::create_dir_all(&release_dir).unwrap();
    let (release, runtime) = release_with_bin(&release_dir, "exit 0");
    let p = placement(release, runtime, 1, true, false);

    let mut exec = EffectExecutor::new(
        NativeDriver::new(),
        tmp.path().join("attempts"),
        AuthorityState::connected(1, 1),
    );
    let err = exec.reconcile(&[p], 0).unwrap_err();
    assert!(matches!(err, AgentError::UnverifiedCapsule));
    assert_eq!(exec.live_count(), 0);
}

#[test]
fn stale_fence_blocks_start_stop_and_report() {
    let tmp = tempfile::tempdir().unwrap();
    let release_dir = tmp.path().join("release");
    fs::create_dir_all(&release_dir).unwrap();
    let (release, runtime) = release_with_bin(&release_dir, "sleep 30");
    let p = placement(release, runtime, 9, false, true);
    let id = p.attempt_id;

    let mut exec = EffectExecutor::new(
        NativeDriver::new(),
        tmp.path().join("attempts"),
        AuthorityState::connected(1, 9),
    );
    exec.reconcile(&[p], 0).unwrap();

    // Controller advances fence → stale relative to placement.
    exec.set_authority(AuthorityState::connected(2, 99));
    let err = exec.cancel(id).unwrap_err();
    assert!(matches!(
        err,
        AgentError::Fence(FenceError::StaleFence { .. })
    ));
    let err = exec.report(id).unwrap_err();
    assert!(matches!(
        err,
        AgentError::Fence(FenceError::StaleFence { .. })
    ));

    // Fresh start with old fence also fails.
    let release_dir2 = tmp.path().join("release2");
    fs::create_dir_all(&release_dir2).unwrap();
    let (release2, runtime2) = release_with_bin(&release_dir2, "exit 0");
    let p2 = placement(release2, runtime2, 9, true, true);
    let err = exec.reconcile(&[p2], 0).unwrap_err();
    assert!(matches!(
        err,
        AgentError::Fence(FenceError::StaleFence { .. })
    ));
}

#[test]
fn isolation_blocks_effects_and_grace_cleans_up() {
    let auth = AuthorityState {
        controller_epoch: 1,
        placement_fence: 4,
        isolated_since_ms: Some(1000),
    };
    assert!(matches!(
        allow_effect(&auth, 4, EffectKind::Start),
        Err(FenceError::Isolated { .. })
    ));
    assert!(matches!(
        allow_effect(&auth, 4, EffectKind::Report),
        Err(FenceError::Isolated { .. })
    ));

    let tmp = tempfile::tempdir().unwrap();
    let release_dir = tmp.path().join("release");
    fs::create_dir_all(&release_dir).unwrap();
    let (release, runtime) = release_with_bin(&release_dir, "sleep 30");
    let p = placement(release, runtime, 4, false, true);
    let id = p.attempt_id;

    let mut exec = EffectExecutor::new(
        NativeDriver::new(),
        tmp.path().join("attempts"),
        AuthorityState::connected(1, 4),
    )
    .with_isolation(IsolationPolicy {
        grace_ms: 50,
        stop_on_isolation: false,
        confirm_window_ms: 10,
    });
    exec.reconcile(std::slice::from_ref(&p), 0).unwrap();
    assert!(exec.attempt_root_exists(id));

    exec.mark_isolated(1_000);
    // While isolated, reconcile returns no reports and does not stop.
    let reports = exec.reconcile(std::slice::from_ref(&p), 1_010).unwrap();
    assert!(reports.is_empty());
    assert_eq!(exec.live_count(), 1);

    // After grace, forced cleanup.
    let err = exec.reconcile(std::slice::from_ref(&p), 1_060).unwrap_err();
    assert!(matches!(err, AgentError::Fence(FenceError::GraceExpired)));
    assert_eq!(exec.live_count(), 0);
    assert!(!exec.attempt_root_exists(id));
}

#[test]
fn stop_on_isolation_uses_short_confirm_window() {
    let policy = IsolationPolicy {
        grace_ms: 15 * 60 * 1000,
        stop_on_isolation: true,
        confirm_window_ms: 25,
    };
    assert_eq!(policy.effective_grace_ms(), 25);

    let tmp = tempfile::tempdir().unwrap();
    let release_dir = tmp.path().join("release");
    fs::create_dir_all(&release_dir).unwrap();
    let (release, runtime) = release_with_bin(&release_dir, "sleep 30");
    let p = placement(release, runtime, 2, false, true);
    let id = p.attempt_id;

    let mut exec = EffectExecutor::new(
        NativeDriver::new(),
        tmp.path().join("attempts"),
        AuthorityState::connected(1, 2),
    )
    .with_isolation(policy);
    exec.reconcile(std::slice::from_ref(&p), 0).unwrap();
    exec.mark_isolated(500);
    let err = exec.reconcile(std::slice::from_ref(&p), 530).unwrap_err();
    assert!(matches!(err, AgentError::Fence(FenceError::GraceExpired)));
    assert!(!exec.attempt_root_exists(id));
}
