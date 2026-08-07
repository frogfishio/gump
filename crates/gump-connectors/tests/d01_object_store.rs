//! D01 exit evidence: overwrite/conflict/fault suite for object connector.
//!
//! Authority: docs/v1/DELIVERY.md D01, docs/v1/RUNTIME.md §13, DECISIONS D008.
//! Capsules remain inert objects — the connector never stores desired state.

use gump_connectors::{
    ByteRange, FakeObjectStore, ObjectStore, ObjectStoreErrorKind, final_capsule_key,
};
use gump_types::{CapsuleId, ClusterId};

fn v7(seed: u8) -> [u8; 16] {
    let mut b = [seed; 16];
    b[6] = (b[6] & 0x0f) | 0x70;
    b[8] = (b[8] & 0x3f) | 0x80;
    b
}

fn ids() -> (ClusterId, CapsuleId) {
    (
        ClusterId::from_bytes(v7(0x11)).unwrap(),
        CapsuleId::from_bytes(v7(0x22)).unwrap(),
    )
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

#[test]
fn quarantine_publish_happy_path() {
    let (cluster, capsule) = ids();
    let body = b"sealed-capsule-bytes";
    let dig = digest(body);
    let mut store = FakeObjectStore::new();

    let upload = store
        .begin_quarantine(cluster, capsule, body.len() as u64)
        .unwrap();
    store.write(upload, body).unwrap();
    let q = store.finish_quarantine(upload, dig).unwrap();
    assert_eq!(q.length, body.len() as u64);
    assert_eq!(q.digest, dig);

    let final_key = final_capsule_key(cluster, capsule).unwrap();
    let published = store
        .publish_if_absent(&q.key, &final_key, dig, body.len() as u64)
        .unwrap();
    assert_eq!(published.key, final_key);

    let head = store.head(&final_key).unwrap();
    assert_eq!(head.digest, dig);
    assert_eq!(store.get(&final_key, None).unwrap(), body);
    assert_eq!(
        store
            .get(
                &final_key,
                Some(ByteRange {
                    start: 0,
                    end: Some(6)
                })
            )
            .unwrap(),
        b"sealed"
    );

    // Store only object keys — no desired-state namespace.
    for key in store.keys() {
        assert!(
            key.as_str().contains("/capsules/") || key.as_str().contains("/quarantine/"),
            "unexpected key {}",
            key
        );
        assert!(!key.as_str().contains("desired"));
        assert!(!key.as_str().contains("workload"));
    }
}

#[test]
fn publish_conflict_on_different_digest() {
    let (cluster, capsule) = ids();
    let a = b"object-a----------------";
    let b = b"object-b-DIFFERENT------";
    assert_ne!(digest(a), digest(b));
    let mut store = FakeObjectStore::new();

    let up = store
        .begin_quarantine(cluster, capsule, a.len() as u64)
        .unwrap();
    store.write(up, a).unwrap();
    let q = store.finish_quarantine(up, digest(a)).unwrap();
    let final_key = final_capsule_key(cluster, capsule).unwrap();
    store
        .publish_if_absent(&q.key, &final_key, digest(a), a.len() as u64)
        .unwrap();

    // Second quarantine with different bytes → conflict on same final key.
    let up2 = store
        .begin_quarantine(cluster, capsule, b.len() as u64)
        .unwrap();
    store.write(up2, b).unwrap();
    let q2 = store.finish_quarantine(up2, digest(b)).unwrap();
    let err = store
        .publish_if_absent(&q2.key, &final_key, digest(b), b.len() as u64)
        .unwrap_err();
    assert_eq!(err.kind(), ObjectStoreErrorKind::Conflict);
}

#[test]
fn publish_idempotent_when_digest_matches() {
    let (cluster, capsule) = ids();
    let body = b"same-bytes";
    let dig = digest(body);
    let mut store = FakeObjectStore::new();
    let up = store
        .begin_quarantine(cluster, capsule, body.len() as u64)
        .unwrap();
    store.write(up, body).unwrap();
    let q = store.finish_quarantine(up, dig).unwrap();
    let final_key = final_capsule_key(cluster, capsule).unwrap();
    store
        .publish_if_absent(&q.key, &final_key, dig, body.len() as u64)
        .unwrap();
    let again = store
        .publish_if_absent(&q.key, &final_key, dig, body.len() as u64)
        .unwrap();
    assert_eq!(again.digest, dig);
}

#[test]
fn abort_and_fault_injection() {
    let (cluster, capsule) = ids();
    let mut store = FakeObjectStore::new();
    let up = store.begin_quarantine(cluster, capsule, 4).unwrap();
    store.abort(up).unwrap();
    assert_eq!(store.open_upload_count(), 0);
    assert_eq!(
        store.write(up, b"x").unwrap_err().kind(),
        ObjectStoreErrorKind::NotFound
    );

    let up2 = store.begin_quarantine(cluster, capsule, 4).unwrap();
    store.faults.fail_next_write = true;
    assert_eq!(
        store.write(up2, b"abcd").unwrap_err().kind(),
        ObjectStoreErrorKind::FaultInjected
    );
    // Fault cleared; write can proceed.
    store.write(up2, b"abcd").unwrap();
    let dig = digest(b"abcd");
    store.faults.fail_next_publish = true;
    let q = store.finish_quarantine(up2, dig).unwrap();
    let final_key = final_capsule_key(cluster, capsule).unwrap();
    assert_eq!(
        store
            .publish_if_absent(&q.key, &final_key, dig, 4)
            .unwrap_err()
            .kind(),
        ObjectStoreErrorKind::FaultInjected
    );

    store.faults.fail_next_head = true;
    assert_eq!(
        store.head(&q.key).unwrap_err().kind(),
        ObjectStoreErrorKind::FaultInjected
    );
    store.faults.fail_next_head = false;
    assert!(store.head(&q.key).is_ok());
}

#[test]
fn finish_rejects_length_or_digest_mismatch() {
    let (cluster, capsule) = ids();
    let mut store = FakeObjectStore::new();
    let up = store.begin_quarantine(cluster, capsule, 4).unwrap();
    store.write(up, b"ab").unwrap();
    assert_eq!(
        store
            .finish_quarantine(up, digest(b"ab"))
            .unwrap_err()
            .kind(),
        ObjectStoreErrorKind::PreconditionFailed
    );

    let up = store.begin_quarantine(cluster, capsule, 2).unwrap();
    store.write(up, b"ab").unwrap();
    assert_eq!(
        store
            .finish_quarantine(up, digest(b"xx"))
            .unwrap_err()
            .kind(),
        ObjectStoreErrorKind::PreconditionFailed
    );
}
