//! STL-01: ClusterState is the sole OpenRaft application state machine.
//!
//! Evidence: two independent machines apply the same RaftCommand sequence and
//! agree; idempotency is stored with the mutation; OpenRaft membership payload
//! updates StoredMembership without a parallel voter ledger in ClusterState.

use gump_memory::{
    Command, Expected, KeyPrefix, RaftCommand, RaftResponse, RecordKey, TypeConfig, ram_v2_stores,
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

#[tokio::test]
async fn two_replicas_agree_on_cluster_state() {
    let (_log_a, mut sm_a) = ram_v2_stores();
    let (_log_b, mut sm_b) = ram_v2_stores();

    let key = RecordKey::new(KeyPrefix::ClusterMeta, "x").unwrap();
    let put = RaftCommand::Record(Command::Put {
        key: key.clone(),
        expected: Expected::Absent,
        payload: b"hello".to_vec(),
        leased: false,
    });
    let acquire = RaftCommand::AcquireController { holder: 7 };
    let desired = RaftCommand::PutDesired {
        namespace: "ns".into(),
        app: "app".into(),
        expected_generation: 0,
        payload: b"decl".to_vec(),
        content_digest: *blake3::hash(b"decl").as_bytes(),
    };
    let op = [1u8; 16];
    let digest = *blake3::hash(b"idem").as_bytes();
    let idem = RaftCommand::Idempotent {
        operation_id: op,
        request_digest: digest,
        inner: Box::new(RaftCommand::Record(Command::Put {
            key: RecordKey::new(KeyPrefix::Names, "n").unwrap(),
            expected: Expected::Absent,
            payload: b"n".to_vec(),
            leased: false,
        })),
    };

    let seq = vec![
        entry(1, put),
        entry(2, acquire),
        entry(3, desired),
        entry(4, idem.clone()),
        entry(5, idem), // replay
    ];

    let ra = RaftStateMachine::apply(&mut sm_a, seq.clone())
        .await
        .unwrap();
    let rb = RaftStateMachine::apply(&mut sm_b, seq).await.unwrap();
    assert_eq!(ra, rb);

    let ca = sm_a.cluster_state().await;
    let cb = sm_b.cluster_state().await;
    assert_eq!(ca.records().revision(), cb.records().revision());
    assert_eq!(ca.desired_generation("ns", "app"), Some(1));
    assert_eq!(cb.desired_generation("ns", "app"), Some(1));
    assert!(ca.controller().current().is_some());
    assert!(matches!(ra.last().unwrap(), RaftResponse::Replay(_)));
}

#[tokio::test]
async fn membership_entry_does_not_mutate_app_voters() {
    use std::collections::BTreeSet;

    let (_log, mut sm) = ram_v2_stores();
    let before = sm.cluster_state().await.records().revision();
    let voters = BTreeSet::from([1u64]);
    let mem = openraft::Membership::new(vec![voters], std::collections::BTreeMap::<u64, ()>::new());
    let entry = Entry {
        log_id: log_id(1),
        payload: EntryPayload::Membership(mem),
    };
    RaftStateMachine::apply(&mut sm, vec![entry]).await.unwrap();
    let (applied, stored) = RaftStateMachine::applied_state(&mut sm).await.unwrap();
    assert!(applied.is_some());
    assert!(
        stored.membership().get_node(&1).is_some()
            || stored.membership().voter_ids().any(|id| id == 1)
    );
    // Application ClusterState revision unchanged by membership-only entry.
    assert_eq!(sm.cluster_state().await.records().revision(), before);
}
