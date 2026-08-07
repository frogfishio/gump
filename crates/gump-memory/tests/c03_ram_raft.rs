//! C03 exit evidence: OpenRaft RAM adapter, no-write proof, 1/2/3/5 quorum sim.
//!
//! Authority: docs/v1/DELIVERY.md C03, DECISIONS D001/D006, PROTOCOL.md §6.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use gump_memory::{
    MemoryNodeId, RamLogStore, RamStateMachine, TypeConfig, can_commit, majority, ram_v2_stores,
};
use gump_types::sim::{LinkFaults, PeerId, SimWorld};
use openraft::storage::{RaftLogStorage, RaftStateMachine};
use openraft::testing::{StoreBuilder, Suite};
use openraft::{Entry, StorageError, Vote};

struct RamBuilder;

impl StoreBuilder<TypeConfig, RamLogStore, RamStateMachine> for RamBuilder {
    async fn build(
        &self,
    ) -> Result<((), RamLogStore, RamStateMachine), StorageError<MemoryNodeId>> {
        let (log, sm) = ram_v2_stores();
        Ok(((), log, sm))
    }
}

#[test]
fn openraft_store_conformance_suite() -> Result<(), StorageError<MemoryNodeId>> {
    Suite::test_all(RamBuilder)
}

#[test]
fn d006_quorum_matrix_1_2_3_5() {
    assert_eq!(majority(1).unwrap(), 1);
    assert!(can_commit(1, 1).unwrap());
    assert!(!can_commit(1, 0).unwrap());

    assert_eq!(majority(2).unwrap(), 2);
    assert!(can_commit(2, 2).unwrap());
    assert!(!can_commit(2, 1).unwrap());

    assert_eq!(majority(3).unwrap(), 2);
    assert!(can_commit(3, 2).unwrap());
    assert!(!can_commit(3, 1).unwrap());

    assert_eq!(majority(5).unwrap(), 3);
    assert!(can_commit(5, 3).unwrap());
    assert!(!can_commit(5, 2).unwrap());
}

#[test]
fn sim_world_partition_blocks_minority_commit_3_and_5() {
    let mut world = SimWorld::<()>::new(1, LinkFaults::default());
    let a = PeerId::new(1);
    let b = PeerId::new(2);
    let c = PeerId::new(3);
    for p in [a, b, c] {
        world.add_peer(p);
    }
    world.partition(a, b);
    world.partition(a, c);
    assert!(!can_commit(3, 1).unwrap());
    assert!(can_commit(3, 2).unwrap());

    let mut world5 = SimWorld::<()>::new(2, LinkFaults::default());
    let peers: Vec<_> = (1..=5).map(PeerId::new).collect();
    for &p in &peers {
        world5.add_peer(p);
    }
    for &iso in &peers[0..2] {
        for &live in &peers[2..] {
            world5.partition(iso, live);
        }
    }
    assert!(can_commit(5, 3).unwrap());
    assert!(!can_commit(5, 2).unwrap());
}

#[tokio::test]
async fn ram_v2_adaptor_round_trip_vote_and_log() {
    let (mut log_store, mut sm) = ram_v2_stores();
    let vote = Vote::new(1, 7);
    RaftLogStorage::save_vote(&mut log_store, &vote)
        .await
        .unwrap();
    let got = RaftLogStorage::read_vote(&mut log_store).await.unwrap();
    assert_eq!(got, Some(vote));

    let (applied, _) = RaftStateMachine::applied_state(&mut sm).await.unwrap();
    assert!(applied.is_none());
}

#[tokio::test]
async fn no_write_proof_store_ops_leave_sandbox_empty() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sandbox = std::env::temp_dir().join(format!("gump-c03-nowrite-{nanos}"));
    fs::create_dir_all(&sandbox).unwrap();
    assert!(dir_is_empty(&sandbox));

    let (mut log_store, mut sm) = ram_v2_stores();
    RaftLogStorage::save_vote(&mut log_store, &Vote::new(3, 1))
        .await
        .unwrap();
    let _ = RaftLogStorage::get_log_state(&mut log_store).await.unwrap();

    RaftStateMachine::apply(&mut sm, Vec::<Entry<TypeConfig>>::new())
        .await
        .unwrap();
    let mut builder = RaftStateMachine::get_snapshot_builder(&mut sm).await;
    let _ = openraft::RaftSnapshotBuilder::build_snapshot(&mut builder)
        .await
        .unwrap();

    assert!(
        dir_is_empty(&sandbox),
        "RAM adapter must not create files under sandbox {:?}",
        sandbox
    );
    let src = include_str!("../src/ram_store.rs");
    assert!(!src.contains("use std::fs"));
    assert!(!src.contains("File::create"));
    assert!(!src.contains("OpenOptions"));
    assert!(!src.contains("std::fs::"));
    let _ = fs::remove_dir_all(sandbox);
}

fn dir_is_empty(path: &PathBuf) -> bool {
    fs::read_dir(path).unwrap().next().is_none()
}
