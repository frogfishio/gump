//! GUMP-N015 / D05: deploy receipt, explain, wait defaults, no false rollback.

use std::sync::{Arc, Mutex};

use gump_cli::{
    DEFAULT_DEPLOY_WAIT, LocalRequest, LocalResponse, normalize_wait_condition, sample_deploy,
    sample_lifecycle,
};
use gump_connectors::FakeObjectStore;
use gump_memory::MemoryCluster;
use gump_server::{InitOptions, LocalDaemon, PeerAllowlist, ProductRuntime, handle_request};

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[test]
fn wait_default_is_intent_accepted() {
    assert_eq!(normalize_wait_condition(None), DEFAULT_DEPLOY_WAIT);
    assert_eq!(
        normalize_wait_condition(Some("accepted")),
        DEFAULT_DEPLOY_WAIT
    );
    assert_eq!(normalize_wait_condition(Some("readiness")), "readiness");
}

#[test]
fn sample_deploy_receipt_distinguishes_stages_and_durability() {
    let LocalResponse::Deploy {
        durability_note,
        stages,
        interrupted_implies_rollback,
        wait,
        ..
    } = sample_deploy()
    else {
        panic!("expected deploy sample");
    };
    assert!(durability_note.contains("zero failure tolerance"));
    assert!(!interrupted_implies_rollback);
    assert_eq!(wait.condition, DEFAULT_DEPLOY_WAIT);
    assert!(wait.matched_default);
    let persist = stages.iter().find(|s| s.name == "persistence").unwrap();
    let intent = stages
        .iter()
        .find(|s| s.name == "intent_acceptance")
        .unwrap();
    let sched = stages.iter().find(|s| s.name == "scheduling").unwrap();
    assert_eq!(persist.status, "completed");
    assert_eq!(intent.status, "completed");
    assert_eq!(sched.status, "pending");
}

#[test]
fn interrupt_does_not_imply_rollback() {
    let LocalResponse::Lifecycle {
        interrupted_implies_rollback,
        note,
        ..
    } = sample_lifecycle()
    else {
        panic!("expected lifecycle");
    };
    assert!(!interrupted_implies_rollback);
    assert!(note.unwrap().contains("not rolled back"));

    let daemon = LocalDaemon::new(PeerAllowlist::same_uid(1));
    let resp = handle_request(
        &daemon,
        LocalRequest::Lifecycle {
            action: "interrupt".into(),
            subject: "attempt/1".into(),
        },
    );
    match resp {
        LocalResponse::Lifecycle {
            interrupted_implies_rollback,
            note,
            ..
        } => {
            assert!(!interrupted_implies_rollback);
            assert!(note.unwrap().contains("does not roll back"));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn explain_reads_committed_view_and_discloses_compaction() {
    let rt = ProductRuntime::init(InitOptions {
        object_store: Some(gump_connectors::RuntimeObjectStore::Memory(
            FakeObjectStore::new(),
        )),
        ..InitOptions::default()
    })
    .unwrap();
    let resp = handle_request(
        &rt.local_api,
        LocalRequest::Explain {
            subject: "unit/demo".into(),
        },
    );
    match resp {
        LocalResponse::Explain {
            observation_source,
            compaction_disclosed,
            durability_note,
            ..
        } => {
            assert_eq!(observation_source, "committed_cluster_memory");
            assert!(compaction_disclosed);
            assert!(durability_note.contains("zero failure tolerance"));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn live_deploy_rejects_bytes_that_are_not_a_verified_capsule() {
    let mut daemon = LocalDaemon::new(PeerAllowlist::same_uid(1));
    daemon.cluster_id = "00000000-0000-4000-8000-000000000099".into();
    daemon.memory_cluster = Some(Arc::new(
        MemoryCluster::bootstrap_one_voter(1, 1).expect("raft"),
    ));
    daemon.object_store = Some(Arc::new(Mutex::new(
        gump_connectors::RuntimeObjectStore::Memory(FakeObjectStore::new()),
    )));

    let body = b"n015-capsule";
    let digest = to_hex(blake3::hash(body).as_bytes());
    let resp = handle_request(
        &daemon,
        LocalRequest::Deploy {
            operation_id: "00000000-0000-4000-8000-0000000000bb".into(),
            namespace: "ns".into(),
            app: "app".into(),
            content_digest_hex: digest.clone(),
            capsule_hex: Some(to_hex(body)),
            capsule_path: None,
            wait: None,
        },
    );
    assert!(matches!(
        resp,
        LocalResponse::Error(ref e) if e.reason == "deploy.bad_capsule"
    ));
}
