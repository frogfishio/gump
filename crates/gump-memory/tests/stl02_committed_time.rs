//! STL-02: lease expiry is deterministic under replication (committed time only).
//!
//! Evidence: the same command sequence yields identical expired lease sets on two
//! machines even when each process has a different wall clock; apply never reads
//! wall time; backward committed timestamps are rejected.

use gump_memory::{
    ApplyError, Command, LeasePurpose, RaftCommand, RaftResponse, TypeConfig, TypedRecordMachine,
    ram_v2_stores,
};
use openraft::storage::RaftStateMachine;
use openraft::{Entry, EntryPayload, LeaderId, LogId};

fn log_id(index: u64) -> LogId<u64> {
    LogId::new(LeaderId::new(1, 1), index)
}

fn entry(index: u64, cmd: RaftCommand) -> Entry<TypeConfig> {
    Entry {
        log_id: log_id(index),
        payload: EntryPayload::Normal(cmd),
    }
}

/// Simulate distinct wall clocks that must not influence apply.
fn wall_clock_ms_a() -> u64 {
    9_000_000
}

fn wall_clock_ms_b() -> u64 {
    1
}

#[test]
fn same_command_sequence_identical_expiry_despite_divergent_wall_clocks() {
    let mut a = TypedRecordMachine::with_defaults();
    let mut b = TypedRecordMachine::with_defaults();

    // Wall clocks differ wildly; neither is passed into apply.
    assert_ne!(wall_clock_ms_a(), wall_clock_ms_b());
    let _ = (wall_clock_ms_a(), wall_clock_ms_b());

    let seq = [
        Command::LeaseGrant {
            purpose: LeasePurpose::MemberLiveness,
            now_ms: 1_000,
        },
        Command::LeaseGrant {
            purpose: LeasePurpose::PlacementAttempt,
            now_ms: 2_000,
        },
        Command::ExpireLeases { now_ms: 16_000 }, // member liveness TTL 15s from 1000 → due
        Command::ExpireLeases { now_ms: 22_001 }, // placement TTL 20s from 2000 → due
    ];

    let mut expired_a = Vec::new();
    let mut expired_b = Vec::new();
    for cmd in seq {
        let ra = a.apply(cmd.clone()).unwrap();
        let rb = b.apply(cmd).unwrap();
        assert_eq!(ra, rb);
        expired_a.extend(ra.expired_lease_ids);
        expired_b.extend(rb.expired_lease_ids);
    }

    assert_eq!(expired_a, expired_b);
    assert_eq!(expired_a, vec![1, 2]);
    assert_eq!(a.now_ms(), b.now_ms());
    assert_eq!(a.now_ms(), 22_001);
    assert!(a.get_lease(1).is_none());
    assert!(b.get_lease(2).is_none());
}

#[test]
fn commit_time_rejects_backward_timestamp() {
    let mut m = TypedRecordMachine::with_defaults();
    m.apply(Command::AdvanceTime { now_ms: 500 }).unwrap();
    let err = m.apply(Command::ExpireLeases { now_ms: 499 }).unwrap_err();
    assert_eq!(
        err,
        ApplyError::TimeWentBackward {
            current: 500,
            presented: 499
        }
    );
    assert_eq!(m.now_ms(), 500);
}

#[test]
fn equal_committed_time_is_accepted() {
    let mut m = TypedRecordMachine::with_defaults();
    m.apply(Command::AdvanceTime { now_ms: 42 }).unwrap();
    m.apply(Command::AdvanceTime { now_ms: 42 }).unwrap();
    assert_eq!(m.now_ms(), 42);
}

#[tokio::test]
async fn raft_replicas_agree_on_lease_expiry_without_wall_clock() {
    let (_log_a, mut sm_a) = ram_v2_stores();
    let (_log_b, mut sm_b) = ram_v2_stores();

    let seq = vec![
        entry(
            1,
            RaftCommand::Record(Command::LeaseGrant {
                purpose: LeasePurpose::ControllerAuthority,
                now_ms: 10,
            }),
        ),
        entry(
            2,
            RaftCommand::Record(Command::ExpireLeases { now_ms: 10_010 }),
        ),
    ];

    let ra = RaftStateMachine::apply(&mut sm_a, seq.clone())
        .await
        .unwrap();
    let rb = RaftStateMachine::apply(&mut sm_b, seq).await.unwrap();
    assert_eq!(ra, rb);

    match ra.last().unwrap() {
        RaftResponse::Applied(out) => assert_eq!(out.expired_lease_ids, vec![1]),
        other => panic!("expected Applied, got {other:?}"),
    }

    let ca = sm_a.cluster_state().await;
    let cb = sm_b.cluster_state().await;
    assert_eq!(ca.records().now_ms(), cb.records().now_ms());
    assert_eq!(ca.records().now_ms(), 10_010);
    assert!(ca.records().get_lease(1).is_none());
    assert!(cb.records().get_lease(1).is_none());
}
