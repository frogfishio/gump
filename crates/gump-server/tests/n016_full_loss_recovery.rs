//! GUMP-N016 / D06: inventory, inspect, reintroduce after full cluster-memory loss.

use std::sync::{Arc, Mutex};

use gump_cli::{LocalRequest, LocalResponse};
use gump_connectors::FakeObjectStore;
use gump_memory::MemoryCluster;
use gump_server::deploy_txn::capsule_id_for_digest;
use gump_server::{LocalDaemon, PeerAllowlist, handle_request};

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn daemon_with_store() -> LocalDaemon {
    let mut daemon = LocalDaemon::new(PeerAllowlist::same_uid(1));
    daemon.cluster_id = "00000000-0000-4000-8000-000000000099".into();
    daemon.memory_cluster = Some(Arc::new(
        MemoryCluster::bootstrap_one_voter(1, 1).expect("raft"),
    ));
    daemon.object_store = Some(Arc::new(Mutex::new(FakeObjectStore::new())));
    daemon
}

fn deploy_body(daemon: &LocalDaemon, body: &[u8], op: &str) -> String {
    let digest = to_hex(blake3::hash(body).as_bytes());
    let resp = handle_request(
        daemon,
        LocalRequest::Deploy {
            operation_id: op.into(),
            namespace: "ns".into(),
            app: "app".into(),
            content_digest_hex: digest.clone(),
            capsule_hex: Some(to_hex(body)),
            wait: None,
        },
    );
    assert!(
        matches!(resp, LocalResponse::Deploy { .. }),
        "deploy failed: {resp:?}"
    );
    digest
}

#[test]
fn empty_cluster_after_restart_has_zero_desired() {
    let mut daemon = daemon_with_store();
    deploy_body(
        &daemon,
        b"n016-before-loss",
        "00000000-0000-4000-8000-0000000000a1",
    );
    assert_eq!(
        daemon
            .memory_cluster
            .as_ref()
            .unwrap()
            .desired_len()
            .unwrap(),
        1
    );

    // Full loss: replace memory cluster (RAM-only). Object store Capsules remain.
    daemon.memory_cluster = Some(Arc::new(
        MemoryCluster::bootstrap_one_voter(1, 1).expect("raft"),
    ));
    assert_eq!(
        daemon
            .memory_cluster
            .as_ref()
            .unwrap()
            .desired_len()
            .unwrap(),
        0
    );

    let LocalResponse::Inventory {
        desired_count,
        capsules,
        ..
    } = handle_request(&daemon, LocalRequest::Inventory)
    else {
        panic!("expected inventory");
    };
    assert_eq!(desired_count, 0);
    assert_eq!(capsules.len(), 1);
    assert!(capsules[0].inert);
    assert!(!capsules[0].live_referenced);
}

#[test]
fn inventory_lists_inert_without_activating() {
    let daemon = daemon_with_store();
    let digest = deploy_body(
        &daemon,
        b"n016-inventory",
        "00000000-0000-4000-8000-0000000000a2",
    );
    let before = daemon
        .memory_cluster
        .as_ref()
        .unwrap()
        .desired_len()
        .unwrap();

    let LocalResponse::Inventory { capsules, .. } =
        handle_request(&daemon, LocalRequest::Inventory)
    else {
        panic!("expected inventory");
    };
    assert_eq!(capsules.len(), 1);
    assert_eq!(capsules[0].content_digest_hex, digest);
    assert!(capsules[0].live_referenced);
    assert_eq!(
        daemon
            .memory_cluster
            .as_ref()
            .unwrap()
            .desired_len()
            .unwrap(),
        before
    );
}

#[test]
fn reintroduce_plan_does_not_mutate_and_apply_requires_finite_mode() {
    let mut daemon = daemon_with_store();
    let body = b"n016-reintroduce";
    deploy_body(&daemon, body, "00000000-0000-4000-8000-0000000000a3");
    let digest = *blake3::hash(body).as_bytes();
    let capsule = capsule_id_for_digest(&digest).to_hyphenated();

    // Lose memory.
    daemon.memory_cluster = Some(Arc::new(
        MemoryCluster::bootstrap_one_voter(1, 1).expect("raft"),
    ));

    let plan = handle_request(
        &daemon,
        LocalRequest::Reintroduce {
            capsule_id: capsule.clone(),
            plan: true,
            finite_mode: Some("new_execution".into()),
            resume_from: None,
            operation_id: None,
            namespace: Some("ns".into()),
            app: Some("app".into()),
        },
    );
    match plan {
        LocalResponse::Reintroduce {
            plan: true,
            restores_prior_desired,
            desired_generation,
            ..
        } => {
            assert!(!restores_prior_desired);
            assert!(desired_generation.is_none());
        }
        other => panic!("expected plan, got {other:?}"),
    }
    assert_eq!(
        daemon
            .memory_cluster
            .as_ref()
            .unwrap()
            .desired_len()
            .unwrap(),
        0
    );

    let missing = handle_request(
        &daemon,
        LocalRequest::Reintroduce {
            capsule_id: capsule.clone(),
            plan: false,
            finite_mode: None,
            resume_from: None,
            operation_id: Some("00000000-0000-4000-8000-0000000000b1".into()),
            namespace: Some("ns".into()),
            app: Some("app".into()),
        },
    );
    assert!(matches!(
        missing,
        LocalResponse::Error(ref e) if e.code == "INVALID_ARGUMENT"
    ));

    let applied = handle_request(
        &daemon,
        LocalRequest::Reintroduce {
            capsule_id: capsule.clone(),
            plan: false,
            finite_mode: Some("new_execution".into()),
            resume_from: None,
            operation_id: Some("00000000-0000-4000-8000-0000000000b2".into()),
            namespace: Some("ns".into()),
            app: Some("app".into()),
        },
    );
    match applied {
        LocalResponse::Reintroduce {
            plan: false,
            phase,
            desired_generation,
            restores_prior_desired,
            ..
        } => {
            assert_eq!(phase, "intent_accepted");
            assert_eq!(desired_generation, Some(1));
            assert!(!restores_prior_desired);
        }
        other => panic!("expected apply, got {other:?}"),
    }
    assert_eq!(
        daemon
            .memory_cluster
            .as_ref()
            .unwrap()
            .desired_len()
            .unwrap(),
        1
    );

    // Only the selected Capsule is activated; a second stored Capsule stays inert.
    // Publish a second Capsule via a distinct workload id, then lose memory again.
    let resp2 = handle_request(
        &daemon,
        LocalRequest::Deploy {
            operation_id: "00000000-0000-4000-8000-0000000000a4".into(),
            namespace: "ns".into(),
            app: "other".into(),
            content_digest_hex: to_hex(blake3::hash(b"n016-other").as_bytes()),
            capsule_hex: Some(to_hex(b"n016-other")),
            wait: None,
        },
    );
    assert!(matches!(resp2, LocalResponse::Deploy { .. }), "{resp2:?}");
    let store = daemon.object_store.clone();
    daemon.memory_cluster = Some(Arc::new(
        MemoryCluster::bootstrap_one_voter(1, 1).expect("raft"),
    ));
    daemon.object_store = store;
    let inv = handle_request(&daemon, LocalRequest::Inventory);
    let LocalResponse::Inventory { capsules, .. } = inv else {
        panic!("inventory");
    };
    assert_eq!(capsules.len(), 2);
    let _ = handle_request(
        &daemon,
        LocalRequest::Reintroduce {
            capsule_id: capsule,
            plan: false,
            finite_mode: Some("new_execution".into()),
            resume_from: None,
            operation_id: Some("00000000-0000-4000-8000-0000000000b3".into()),
            namespace: Some("ns".into()),
            app: Some("app".into()),
        },
    );
    assert_eq!(
        daemon
            .memory_cluster
            .as_ref()
            .unwrap()
            .desired_len()
            .unwrap(),
        1,
        "only selected Capsule receives fresh intent"
    );
}

#[test]
fn missing_or_unknown_capsule_stays_inert() {
    let daemon = daemon_with_store();
    let resp = handle_request(
        &daemon,
        LocalRequest::Reintroduce {
            capsule_id: "00000000-0000-4000-8000-0000000000ff".into(),
            plan: false,
            finite_mode: Some("new_execution".into()),
            resume_from: None,
            operation_id: Some("00000000-0000-4000-8000-0000000000c1".into()),
            namespace: None,
            app: None,
        },
    );
    assert!(matches!(
        resp,
        LocalResponse::Error(ref e) if e.code == "NOT_FOUND"
    ));
    assert_eq!(
        daemon
            .memory_cluster
            .as_ref()
            .unwrap()
            .desired_len()
            .unwrap(),
        0
    );
}
