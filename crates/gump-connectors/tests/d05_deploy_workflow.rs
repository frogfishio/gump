//! D05 exit evidence: deploy workflow acceptance matrix.
//!
//! Authority: DELIVERY D05, CLI_LIFECYCLE.md §3/§9, PROTOCOL.md §13–§15,
//! CONFORMANCE Deploy receipt, DECISIONS D014.

use gump_connectors::{
    default_wait_condition, format_receipt_human, ConvergenceSnapshot, DeployBackend,
    DeployFailure, DeployOutcome, DeployPhase, DeployRequest, DeployWorkflow, ObjectKey,
    ObjectLocator, WaitCondition, WorkloadContract,
};
use gump_types::{CapsuleId, WorkloadId};

fn v7(seed: u8) -> [u8; 16] {
    let mut b = [seed; 16];
    b[6] = (b[6] & 0x0f) | 0x70;
    b[8] = (b[8] & 0x3f) | 0x80;
    b
}

fn capsule() -> CapsuleId {
    CapsuleId::from_bytes(v7(0x41)).unwrap()
}

fn workload() -> WorkloadId {
    WorkloadId::from_bytes(v7(0x42)).unwrap()
}

fn op(n: u8) -> [u8; 16] {
    let mut id = [0u8; 16];
    id[15] = n;
    id
}

fn contract_finite() -> WorkloadContract {
    WorkloadContract {
        app_name: "accounts-service".into(),
        lifecycle_finite: true,
        declares_readiness: false,
        requires_publication: false,
        is_gang: false,
        units: 1,
    }
}

fn contract_continuous_ready() -> WorkloadContract {
    WorkloadContract {
        app_name: "api".into(),
        lifecycle_finite: false,
        declares_readiness: true,
        requires_publication: false,
        is_gang: false,
        units: 3,
    }
}

fn contract_published() -> WorkloadContract {
    WorkloadContract {
        app_name: "edge".into(),
        lifecycle_finite: false,
        declares_readiness: true,
        requires_publication: true,
        is_gang: false,
        units: 2,
    }
}

fn locator() -> ObjectLocator {
    ObjectLocator {
        key: ObjectKey::new("clusters/x/capsules/y.capsule").unwrap(),
        uri: "s3://gump/clusters/x/capsules/y.capsule".into(),
    }
}

#[derive(Clone, Copy, Debug)]
enum FailAt {
    None,
    Publish,
    Accept,
    ObserveLost,
}

struct FakeBackend {
    fail_at: FailAt,
    publish_count: u32,
    accept_count: u32,
    observe_count: u32,
    /// After first accept failure, succeed on retry (SAME_OPERATION resume).
    accept_fails_remaining: u32,
}

impl FakeBackend {
    fn ok() -> Self {
        Self {
            fail_at: FailAt::None,
            publish_count: 0,
            accept_count: 0,
            observe_count: 0,
            accept_fails_remaining: 0,
        }
    }

    fn fail(at: FailAt) -> Self {
        Self {
            fail_at: at,
            ..Self::ok()
        }
    }
}

impl DeployBackend for FakeBackend {
    fn publish_capsule(
        &mut self,
        _capsule_id: CapsuleId,
        _capsule_digest: [u8; 32],
        _operation_id: [u8; 16],
    ) -> Result<ObjectLocator, DeployFailure> {
        self.publish_count += 1;
        if matches!(self.fail_at, FailAt::Publish) {
            return Err(DeployFailure {
                phase: DeployPhase::ImmutablePublish,
                reason: "put-if-absent conflict".into(),
                orphan: None,
            });
        }
        Ok(locator())
    }

    fn accept_intent(
        &mut self,
        _operation_id: [u8; 16],
        _capsule_id: CapsuleId,
        _capsule_digest: [u8; 32],
    ) -> Result<(WorkloadId, u64, u64), DeployFailure> {
        self.accept_count += 1;
        if self.accept_fails_remaining > 0 {
            self.accept_fails_remaining -= 1;
            return Err(DeployFailure {
                phase: DeployPhase::IntentAccept,
                reason: "kv quorum timeout".into(),
                orphan: None,
            });
        }
        if matches!(self.fail_at, FailAt::Accept) {
            return Err(DeployFailure {
                phase: DeployPhase::IntentAccept,
                reason: "kv write failed".into(),
                orphan: None,
            });
        }
        Ok((workload(), 1, 1842))
    }

    fn observe(
        &mut self,
        wait: WaitCondition,
        workload_id: WorkloadId,
        generation: u64,
    ) -> Result<ConvergenceSnapshot, DeployFailure> {
        self.observe_count += 1;
        if matches!(self.fail_at, FailAt::ObserveLost) {
            return Ok(ConvergenceSnapshot {
                wait,
                satisfied: false,
                units_eligible: 0,
                units_total: 1,
                workload_id,
                generation,
                cluster_revision: 1842,
                memory_members: 1,
            });
        }
        Ok(ConvergenceSnapshot {
            wait,
            satisfied: true,
            units_eligible: 1,
            units_total: 1,
            workload_id,
            generation,
            cluster_revision: 1842,
            memory_members: 1,
        })
    }
}

fn req(contract: WorkloadContract, operation_id: [u8; 16], wait: Option<WaitCondition>) -> DeployRequest {
    DeployRequest {
        operation_id,
        principal: "oidc:deployer".into(),
        contract,
        capsule_id: capsule(),
        capsule_digest: [0xab; 32],
        wait,
        memory_members: 1,
    }
}

#[test]
fn default_wait_matrix_d014() {
    assert_eq!(
        default_wait_condition(&contract_finite()),
        WaitCondition::Completed
    );
    assert_eq!(
        default_wait_condition(&contract_continuous_ready()),
        WaitCondition::Eligible
    );
    assert_eq!(
        default_wait_condition(&contract_published()),
        WaitCondition::Published
    );
    let continuous_started = WorkloadContract {
        declares_readiness: false,
        requires_publication: false,
        ..contract_continuous_ready()
    };
    assert_eq!(
        default_wait_condition(&continuous_started),
        WaitCondition::Started
    );
}

#[test]
fn successful_finite_deploy_receipt_is_truthful() {
    let mut wf = DeployWorkflow::new();
    let mut backend = FakeBackend::ok();
    let outcome = wf.run(
        &mut backend,
        req(contract_finite(), op(1), None),
        1_000,
    );
    let DeployOutcome::Success(receipt) = outcome else {
        panic!("expected success: {outcome:?}");
    };
    assert_eq!(receipt.wait, WaitCondition::Completed);
    assert_eq!(receipt.generation, 1);
    assert_eq!(receipt.cluster_revision, 1842);
    let human = format_receipt_human(&receipt);
    assert!(human.contains("Application: accounts-service"));
    assert!(human.contains("Intent:      accepted at cluster revision 1842"));
    assert!(human.contains("zero failure tolerance"));
    assert!(!human.to_lowercase().contains("deployed"));
    assert!(human.contains("persisted in s3://"));
}

#[test]
fn publish_failure_has_no_orphan() {
    let mut wf = DeployWorkflow::new();
    let mut backend = FakeBackend::fail(FailAt::Publish);
    let outcome = wf.run(
        &mut backend,
        req(contract_finite(), op(2), Some(WaitCondition::Accepted)),
        1_000,
    );
    let DeployOutcome::Failed(fail) = outcome else {
        panic!("expected failure");
    };
    assert_eq!(fail.phase, DeployPhase::ImmutablePublish);
    assert!(fail.orphan.is_none());
    assert!(wf.orphans.is_empty());
    assert_eq!(backend.accept_count, 0);
}

#[test]
fn accept_failure_reports_orphan_capsule() {
    let mut wf = DeployWorkflow::new();
    let mut backend = FakeBackend::fail(FailAt::Accept);
    let outcome = wf.run(
        &mut backend,
        req(contract_finite(), op(3), Some(WaitCondition::Accepted)),
        1_000,
    );
    let DeployOutcome::Failed(fail) = outcome else {
        panic!("expected failure");
    };
    assert_eq!(fail.phase, DeployPhase::IntentAccept);
    let orphan = fail.orphan.expect("orphan");
    assert_eq!(orphan.capsule_id, capsule());
    assert_eq!(wf.orphans.len(), 1);
    assert_eq!(backend.publish_count, 1);
}

#[test]
fn retry_same_operation_after_accept_failure_does_not_duplicate_intent() {
    let mut wf = DeployWorkflow::new();
    let mut backend = FakeBackend {
        fail_at: FailAt::None,
        accept_fails_remaining: 1,
        ..FakeBackend::ok()
    };
    let r = req(contract_finite(), op(4), Some(WaitCondition::Accepted));
    let first = wf.run(&mut backend, r.clone(), 1_000);
    assert!(matches!(first, DeployOutcome::Failed(_)));
    assert_eq!(wf.orphans.len(), 1);

    let second = wf.run(&mut backend, r, 1_001);
    let DeployOutcome::Success(receipt) = second else {
        panic!("retry should succeed: {second:?}");
    };
    assert_eq!(receipt.operation_id, op(4));
    // publish may run again; accept exactly once successfully after one failure
    assert_eq!(backend.accept_count, 2);
    assert_eq!(backend.publish_count, 2);
}

#[test]
fn retry_same_operation_replays_cached_success() {
    let mut wf = DeployWorkflow::new();
    let mut backend = FakeBackend::ok();
    let r = req(contract_finite(), op(5), Some(WaitCondition::Accepted));
    let first = wf.run(&mut backend, r.clone(), 1_000);
    assert!(matches!(first, DeployOutcome::Success(_)));
    let publishes = backend.publish_count;
    let accepts = backend.accept_count;

    let second = wf.run(&mut backend, r, 1_100);
    assert!(matches!(second, DeployOutcome::Success(_)));
    assert_eq!(backend.publish_count, publishes);
    assert_eq!(backend.accept_count, accepts);
}

#[test]
fn different_request_same_operation_id_conflicts() {
    let mut wf = DeployWorkflow::new();
    let mut backend = FakeBackend::ok();
    let mut r = req(contract_finite(), op(6), Some(WaitCondition::Accepted));
    assert!(matches!(
        wf.run(&mut backend, r.clone(), 1_000),
        DeployOutcome::Success(_)
    ));
    r.capsule_digest = [0xcd; 32];
    let outcome = wf.run(&mut backend, r, 1_100);
    assert!(matches!(
        outcome,
        DeployOutcome::Conflict {
            operation_id
        } if operation_id == op(6)
    ));
}

#[test]
fn wait_observation_loss_before_condition() {
    let mut wf = DeployWorkflow::new();
    let mut backend = FakeBackend::fail(FailAt::ObserveLost);
    let outcome = wf.run(
        &mut backend,
        req(contract_finite(), op(7), None), // default Completed
        1_000,
    );
    let DeployOutcome::Failed(fail) = outcome else {
        panic!("expected wait loss");
    };
    assert_eq!(fail.phase, DeployPhase::WaitObservation);
    assert!(fail.orphan.is_none());
}

#[test]
fn accepted_wait_skips_unit_convergence() {
    let mut wf = DeployWorkflow::new();
    let mut backend = FakeBackend::fail(FailAt::ObserveLost);
    let outcome = wf.run(
        &mut backend,
        req(contract_finite(), op(8), Some(WaitCondition::Accepted)),
        1_000,
    );
    let DeployOutcome::Success(receipt) = outcome else {
        panic!("accepted wait should succeed: {outcome:?}");
    };
    assert_eq!(receipt.wait, WaitCondition::Accepted);
}

#[test]
fn workflow_acceptance_matrix_phases() {
    // CLI_LIFECYCLE §9 outcome taxonomy — phase discrimination.
    let cases: &[(&str, FailAt, Option<DeployPhase>)] = &[
        ("capsule persistence / publish", FailAt::Publish, Some(DeployPhase::ImmutablePublish)),
        ("live-intent acceptance", FailAt::Accept, Some(DeployPhase::IntentAccept)),
        ("lost observation", FailAt::ObserveLost, Some(DeployPhase::WaitObservation)),
        ("converged", FailAt::None, None),
    ];
    for (i, (label, fail_at, expect_phase)) in cases.iter().enumerate() {
        let mut wf = DeployWorkflow::new();
        let mut backend = FakeBackend::fail(*fail_at);
        let outcome = wf.run(
            &mut backend,
            req(
                contract_finite(),
                op(0x20 + i as u8),
                if matches!(fail_at, FailAt::None) {
                    Some(WaitCondition::Accepted)
                } else {
                    None
                },
            ),
            2_000,
        );
        match (expect_phase, outcome) {
            (None, DeployOutcome::Success(_)) => {}
            (Some(phase), DeployOutcome::Failed(f)) => {
                assert_eq!(&f.phase, phase, "{label}");
            }
            (expect, got) => panic!("{label}: expect {expect:?} got {got:?}"),
        }
    }
}

#[test]
fn finite_never_defaults_to_publication_wait() {
    let c = WorkloadContract {
        requires_publication: true, // undeclared for finite — still Completed default
        ..contract_finite()
    };
    // D014: finite → completed regardless of publication flag on this helper.
    // CLI invariant 7: finite never waits for undeclared service condition.
    // Our default_wait prefers finite → Completed first.
    assert_eq!(default_wait_condition(&c), WaitCondition::Completed);
}
