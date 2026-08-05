//! C05 exit evidence: watches, compaction, lease expiry.
//!
//! Authority: docs/v1/DELIVERY.md C05, PROTOCOL.md §8.

use gump_memory::{
    Command, Compacted, Expected, KeyPrefix, LeasePurpose, RecordKey, TypedRecordMachine,
    WatchChange, MAX_WATCH_AGE_MS,
};

fn key(prefix: KeyPrefix, suffix: &str) -> RecordKey {
    RecordKey::new(prefix, suffix).unwrap()
}

fn put(m: &mut TypedRecordMachine, k: &RecordKey, payload: &[u8]) {
    m.apply(Command::Put {
        key: k.clone(),
        expected: Expected::Any,
        payload: payload.to_vec(),
        leased: false,
    })
    .unwrap();
}

#[test]
fn watch_after_returns_ordered_committed_changes() {
    let mut m = TypedRecordMachine::with_defaults();
    let k = key(KeyPrefix::Names, "a");
    put(&mut m, &k, b"v1");
    put(&mut m, &k, b"v2");

    let batches = m.watch_after(0).unwrap();
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].revision, 1);
    assert_eq!(batches[1].revision, 2);
    assert!(matches!(
        &batches[1].changes[0],
        WatchChange::Put { revision: 2, .. }
    ));

    let only_second = m.watch_after(1).unwrap();
    assert_eq!(only_second.len(), 1);
    assert_eq!(only_second[0].revision, 2);
}

#[test]
fn compact_makes_lagging_watcher_receive_compacted() {
    let mut m = TypedRecordMachine::with_defaults();
    let k = key(KeyPrefix::ClusterMeta, "");
    put(&mut m, &k, b"a");
    put(&mut m, &k, b"b");
    put(&mut m, &k, b"c");
    assert_eq!(m.revision(), 3);

    m.apply(Command::Compact { through: 2 }).unwrap();
    assert_eq!(m.compaction_floor(), 2);

    assert_eq!(
        m.watch_after(0).unwrap_err(),
        Compacted {
            compaction_floor: 2
        }
    );
    assert_eq!(
        m.watch_after(1).unwrap_err(),
        Compacted {
            compaction_floor: 2
        }
    );

    // Resume from compaction floor: revision 3 is still available.
    let resumed_pre = m.watch_after(2).unwrap();
    assert_eq!(resumed_pre.len(), 1);
    assert_eq!(resumed_pre[0].revision, 3);

    put(&mut m, &k, b"d");
    let resumed = m.watch_after(2).unwrap();
    assert_eq!(resumed.len(), 2);
    assert_eq!(resumed[1].revision, 4);
}

#[test]
fn slow_watch_age_retention_compacts_history() {
    let mut m = TypedRecordMachine::with_defaults();
    m.set_now_ms(1_000);
    let k = key(KeyPrefix::Names, "slow");
    put(&mut m, &k, b"old");
    assert_eq!(m.watch_after(0).unwrap().len(), 1);

    // Age retention drops the old batch and raises the compaction floor.
    m.advance_now_ms(MAX_WATCH_AGE_MS + 1);

    let floor = m.compaction_floor();
    assert_eq!(floor, 1, "aged revision 1 must be compacted away");
    assert_eq!(
        m.watch_after(0).unwrap_err().compaction_floor,
        floor,
        "lagging watcher must see COMPACTED"
    );

    // Resume from compaction floor, then observe the next put.
    assert!(m.watch_after(floor).unwrap().is_empty());
    put(&mut m, &k, b"new");
    let catch_up = m.watch_after(floor).unwrap();
    assert_eq!(catch_up.len(), 1);
    assert_eq!(catch_up[0].revision, m.revision());
    assert!(matches!(
        &catch_up[0].changes[0],
        WatchChange::Put { .. }
    ));
}

#[test]
fn lease_grant_renew_and_expiry_simulation() {
    let mut m = TypedRecordMachine::with_defaults();
    m.set_now_ms(100);

    let grant = m
        .apply(Command::LeaseGrant {
            purpose: LeasePurpose::MemberLiveness,
        })
        .unwrap();
    let lease = grant.lease.unwrap();
    assert_eq!(lease.ttl_ms, 15_000);
    assert_eq!(lease.expires_at_ms, 15_100);
    let id = lease.id;
    let after_grant = grant.revision;

    m.advance_now_ms(5_000);
    let renewed = m.apply(Command::LeaseRenew { lease_id: id }).unwrap();
    assert_eq!(renewed.lease.unwrap().expires_at_ms, 100 + 5_000 + 15_000);

    m.advance_now_ms(20_000);
    let expired = m.apply(Command::ExpireLeases).unwrap();
    assert_eq!(expired.expired_lease_ids, vec![id]);
    assert!(m.get_lease(id).is_none());

    let batches = m.watch_after(after_grant).unwrap();
    assert!(batches.iter().any(|b| {
        b.changes
            .iter()
            .any(|c| matches!(c, WatchChange::LeaseRevoked { lease_id, .. } if *lease_id == id))
    }));
}

#[test]
fn lease_revoke_is_watchable() {
    let mut m = TypedRecordMachine::with_defaults();
    m.set_now_ms(0);
    let id = m
        .apply(Command::LeaseGrant {
            purpose: LeasePurpose::PlacementAttempt,
        })
        .unwrap()
        .lease
        .unwrap()
        .id;

    let before = m.revision();
    m.apply(Command::LeaseRevoke { lease_id: id }).unwrap();
    let batches = m.watch_after(before).unwrap();
    assert!(matches!(
        batches[0].changes[0],
        WatchChange::LeaseRevoked { lease_id, .. } if lease_id == id
    ));
}

#[test]
fn default_lease_ttls_match_protocol() {
    assert_eq!(LeasePurpose::ControllerAuthority.ttl_ms(), 10_000);
    assert_eq!(LeasePurpose::ControllerAuthority.renew_by_ms(), 3_000);
    assert_eq!(LeasePurpose::GangReservation.ttl_ms(), 30_000);
    assert_eq!(LeasePurpose::TelemetrySubscription.renew_by_ms(), 10_000);
}
