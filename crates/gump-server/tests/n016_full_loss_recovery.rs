//! GUMP-N016 / D06: inventory, inspect, reintroduce after full cluster-memory loss.

use std::sync::{Arc, Mutex};

use gump_capsule::{GumpCapsuleHeader, write_gump_capsule};
use gump_cli::{LocalRequest, LocalResponse};
use gump_connectors::{FakeObjectStore, ObjectStore, final_capsule_key};
use gump_crypto::{
    SegmentDigestRef, SignerEnrollment, SignerTrustPolicy, SigningKeyBytes,
    build_release_signing_transcript, ed25519_fingerprint, sign_transcript, verifying_key,
};
use gump_memory::{MemoryCluster, RaftCommand};
use gump_server::deploy_txn::parse_cluster_id;
use gump_server::{LocalDaemon, PeerAllowlist, handle_request};
use gump_types::CapsuleId;

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
    daemon.object_store = Some(Arc::new(Mutex::new(
        gump_connectors::RuntimeObjectStore::Memory(FakeObjectStore::new()),
    )));
    let mut trust = SignerTrustPolicy::new();
    trust
        .enroll(SignerEnrollment {
            public_key: verifying_key(&SigningKeyBytes::from_bytes([0x61; 32])),
            namespaces: std::collections::BTreeSet::from(["ns".into()]),
            expires_at_ms: None,
            capabilities: std::collections::BTreeSet::new(),
        })
        .unwrap();
    daemon.signer_trust = Arc::new(trust);
    daemon
}

fn signed_capsule(cluster: gump_types::ClusterId, capsule: CapsuleId, archive: &[u8]) -> Vec<u8> {
    let signing = SigningKeyBytes::from_bytes([0x61; 32]);
    let verifying = verifying_key(&signing);
    let header = GumpCapsuleHeader {
        capsule_id: *capsule.as_bytes(),
        cluster_id: *cluster.as_bytes(),
        release_signer: ed25519_fingerprint(&verifying.0)
            .strip_prefix("blake3:")
            .unwrap()
            .into(),
        created_unix_ms: 0,
    };
    let placeholder = [0u8; 96];
    let segments = [
        b"meta".as_slice(),
        archive,
        b"protected",
        b"envelope",
        &placeholder,
    ];
    let logical = [4, archive.len() as u64, 9, 8, 0];
    let mut provisional = Vec::new();
    let view = write_gump_capsule(&mut provisional, &header, segments, logical).unwrap();
    let refs = [0usize, 1, 2, 3].map(|i| SegmentDigestRef {
        segment_type: (i + 1) as u16,
        stored_length: view.table.descriptors[i].stored_length,
        digest: view.table.descriptors[i].digest,
    });
    let transcript =
        build_release_signing_transcript(&header.encode_cbor().unwrap(), 1, &refs).unwrap();
    let signature = sign_transcript(&signing, &transcript).unwrap();
    let mut signature_segment = verifying.0.to_vec();
    signature_segment.extend_from_slice(&signature);
    let final_segments = [
        b"meta".as_slice(),
        archive,
        b"protected",
        b"envelope",
        signature_segment.as_slice(),
    ];
    let mut out = Vec::new();
    write_gump_capsule(&mut out, &header, final_segments, logical).unwrap();
    out
}

fn deploy_body(daemon: &LocalDaemon, body: &[u8], op: &str) -> (String, CapsuleId) {
    let cluster_id = parse_cluster_id(&daemon.cluster_id);
    let mut id = *cluster_id.as_bytes();
    id[14] ^= body.first().copied().unwrap_or(1);
    id[15] ^= body.last().copied().unwrap_or(1);
    id[6] = (id[6] & 0x0f) | 0x70;
    id[8] = (id[8] & 0x3f) | 0x80;
    let capsule = CapsuleId::from_bytes(id).unwrap();
    let bytes = signed_capsule(cluster_id, capsule, body);
    let digest = *blake3::hash(&bytes).as_bytes();
    let store = daemon.object_store.as_ref().unwrap();
    let mut store = store.lock().unwrap();
    let upload = store
        .begin_quarantine(cluster_id, capsule, bytes.len() as u64)
        .unwrap();
    store.write(upload, &bytes).unwrap();
    let q = store.finish_quarantine(upload, digest).unwrap().key;
    let final_key = final_capsule_key(cluster_id, capsule).unwrap();
    store
        .publish_if_absent(&q, &final_key, digest, bytes.len() as u64)
        .unwrap();
    drop(store);
    daemon
        .memory_cluster
        .as_ref()
        .unwrap()
        .client_write(RaftCommand::PutDesired {
            namespace: "ns".into(),
            app: if body == b"n016-other" {
                "other"
            } else {
                "app"
            }
            .into(),
            expected_generation: 0,
            payload: op.as_bytes().to_vec(),
            content_digest: digest,
        })
        .unwrap();
    (to_hex(&digest), capsule)
}

#[test]
fn empty_cluster_after_restart_has_zero_desired() {
    let mut daemon = daemon_with_store();
    let _ = deploy_body(
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
    let (digest, _) = deploy_body(
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
    let (_, capsule) = deploy_body(&daemon, body, "00000000-0000-4000-8000-0000000000a3");
    let capsule = capsule.to_hyphenated();

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
    let _ = deploy_body(
        &daemon,
        b"n016-other",
        "00000000-0000-4000-8000-0000000000a4",
    );
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
