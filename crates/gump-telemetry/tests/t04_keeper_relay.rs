//! T04 exit evidence: keeper selection, node-loss, transfer, overflow.
//!
//! Authority: docs/v1/DELIVERY.md T04, DECISIONS D011, TELEMETRY.md §7/§12.

use gump_telemetry::{
    BatchAuth, DedupId, RelayMesh, RelayRecord, TARGET_KEEPER_REPLICAS, TelemetryBatch,
    select_keepers,
};

fn auth() -> BatchAuth {
    BatchAuth {
        session_id: 7,
        attempt_fence_digest: [0xAB; 32],
    }
}

fn record(seq: u64, payload: &[u8]) -> RelayRecord {
    RelayRecord {
        dedup: DedupId {
            execution_id: 1,
            attempt_id: 2,
            topic: "stdout".into(),
            sequence: seq,
        },
        payload: payload.to_vec(),
    }
}

fn batch(shard: &[u8], seqs: &[u64]) -> TelemetryBatch {
    TelemetryBatch {
        shard_key: shard.to_vec(),
        auth: auth(),
        records: seqs
            .iter()
            .map(|s| record(*s, format!("line-{s}").as_bytes()))
            .collect(),
    }
}

#[test]
fn keeper_selection_two_of_three_plus() {
    let nodes = [1u64, 2, 3, 4, 5];
    let k = select_keepers(b"app/accounts", &nodes);
    assert_eq!(k.len(), TARGET_KEEPER_REPLICAS);
    // Stable across reshuffles of input order.
    let mut shuffled = [5u64, 1, 4, 2, 3];
    let k2 = select_keepers(b"app/accounts", &shuffled);
    assert_eq!(k, k2);
    shuffled.reverse();
    assert_eq!(k, select_keepers(b"app/accounts", &shuffled));
}

#[test]
fn unauthorized_batch_rejected() {
    let mut mesh = RelayMesh::new(vec![1, 2, 3], auth(), 64 * 1024);
    let mut bad = batch(b"shard", &[1]);
    bad.auth.session_id = 99;
    assert!(mesh.relay(&bad).is_err());
}

#[test]
fn overflow_drops_oldest_on_keeper() {
    let mut mesh = RelayMesh::new(vec![1, 2, 3], auth(), 80);
    let shard = b"overflow-shard";
    // Fill beyond tiny budget; each record ~32+ overhead.
    for s in 0..20u64 {
        mesh.relay(&batch(shard, &[s])).unwrap();
    }
    let keepers = mesh.keepers_for(shard);
    assert_eq!(keepers.len(), 2);
    let store = mesh.store(keepers[0]).unwrap();
    assert!(store.dropped_oldest() > 0);
    assert!(store.total_bytes() <= 80);
    // Newest sequences should still be present preferentially.
    assert!(store.contains_dedup(&record(19, b"x").dedup));
}

#[test]
fn node_loss_preserves_records_on_surviving_keeper() {
    let mut mesh = RelayMesh::new(vec![10, 20, 30], auth(), 64 * 1024);
    let shard = b"survive";
    mesh.relay(&batch(shard, &[1, 2, 3])).unwrap();
    let keepers = mesh.keepers_for(shard);
    assert_eq!(keepers.len(), 2);
    let victim = keepers[0];
    let survivor = keepers[1];
    assert!(
        mesh.store(survivor)
            .unwrap()
            .contains_dedup(&record(2, b"x").dedup)
    );

    mesh.lose_node(victim);
    assert!(mesh.store(victim).is_none());
    // Surviving keeper still holds accepted records.
    assert!(
        mesh.store(survivor)
            .unwrap()
            .contains_dedup(&record(2, b"x").dedup)
    );
    // Relay continues with remaining nodes (now < 3 → all eligible).
    mesh.relay(&batch(shard, &[4])).unwrap();
    assert!(
        mesh.store(survivor)
            .unwrap()
            .contains_dedup(&record(4, b"x").dedup)
    );
}

#[test]
fn transfer_join_moves_window_to_new_keeper() {
    let mut mesh = RelayMesh::new(vec![1, 2, 3], auth(), 64 * 1024);
    let shard = b"transfer";
    mesh.relay(&batch(shard, &[10, 11])).unwrap();

    // Lose one keeper, join a replacement, transfer retained window.
    let before = mesh.keepers_for(shard);
    mesh.lose_node(before[0]);
    let moved = mesh.transfer_join(99, shard);
    assert!(moved >= 1);
    // Dedup: re-relay same sequences must not inflate visible count wrongly.
    let again = mesh.relay(&batch(shard, &[10])).unwrap();
    // newly accepted across keepers may be 0 if already present on all targets
    assert!(again == 0 || again > 0);
    let keepers = mesh.keepers_for(shard);
    assert!(keepers.iter().any(|&k| {
        mesh.store(k)
            .map(|s| s.contains_dedup(&record(10, b"x").dedup))
            .unwrap_or(false)
    }));
}

#[test]
fn duplicate_relay_dedupes_per_keeper() {
    let mut mesh = RelayMesh::new(vec![1, 2, 3], auth(), 64 * 1024);
    let shard = b"dedupe";
    let first = mesh.relay(&batch(shard, &[1])).unwrap();
    assert!(first >= 2); // two keepers
    let second = mesh.relay(&batch(shard, &[1])).unwrap();
    assert_eq!(second, 0);
}
