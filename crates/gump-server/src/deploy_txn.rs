//! One-server deploy transaction (GUMP-N010 / DELIVERY D05).
//!
//! Upload → immutable publish → authorized intent accept → observation, with
//! **Raft** `RaftCommand::Idempotent` as the sole authoritative idempotency
//! store. Process-local `IdempotencyCache` in connectors is not used here.

#![allow(clippy::result_large_err)]

use std::io::Cursor;

use gump_capsule::{SegmentType, StreamingCapsuleReader};
use gump_connectors::{
    ByteRange, DeployPhase, ObjectLocator, ObjectStore, ObjectStoreErrorKind, OrphanCapsule,
    StreamedIngress, final_capsule_key,
};
use gump_crypto::SignerTrustPolicy;
use gump_memory::{MemoryCluster, RaftCommand, RaftResponse};
use gump_protocol::pb::ReleaseMetadataV1;
use gump_types::{CapsuleId, ClusterId};
use prost::Message;
use serde::{Deserialize, Serialize};

/// Versioned pointer committed into RAM desired state. Executable declarations
/// and secrets are never copied here; the controller opens the verified,
/// immutable Capsule identified by this record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DesiredCapsuleBindingV1 {
    pub schema: String,
    pub operation_id: String,
    pub capsule_id: String,
}

/// Inputs for one deploy attempt on the local daemon.
#[derive(Clone, Debug)]
pub struct DeployTxnRequest {
    pub operation_id: [u8; 16],
    pub operation_id_display: String,
    pub namespace: String,
    pub app: String,
    /// Compare-and-set generation observed immediately before intent commit.
    pub expected_generation: u64,
    pub content_digest: [u8; 32],
    /// Sealed Capsule bytes for quarantine→publish. Optional when the final
    /// object already exists with a matching digest (idempotent republish).
    pub capsule_bytes: Option<Vec<u8>>,
    pub cluster_id: ClusterId,
    pub capsule_id: CapsuleId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeployTxnOutcome {
    Success {
        desired_generation: u64,
        cluster_revision: u64,
        object: ObjectLocator,
        replayed: bool,
    },
    Conflict {
        operation_id: [u8; 16],
    },
    Failed {
        phase: DeployPhase,
        reason: String,
        orphan: Option<OrphanCapsule>,
    },
}

/// Digest bound into Raft `Idempotent` (PROTOCOL.md §15 / D014).
pub fn deploy_request_digest(req: &DeployTxnRequest) -> [u8; 32] {
    let mut buf = Vec::with_capacity(16 + 32 + 8 + req.namespace.len() + req.app.len() + 2);
    buf.extend_from_slice(&req.operation_id);
    buf.extend_from_slice(&req.content_digest);
    buf.extend_from_slice(req.namespace.as_bytes());
    buf.push(0);
    buf.extend_from_slice(req.app.as_bytes());
    buf.extend_from_slice(&req.expected_generation.to_be_bytes());
    *blake3::hash(&buf).as_bytes()
}

/// Parse a human operation id into wire bytes (UUIDv7 when possible, else hash).
pub fn parse_operation_id(s: &str) -> Result<[u8; 16], String> {
    if let Ok(id) = s.parse::<CapsuleId>() {
        return Ok(*id.as_bytes());
    }
    if s.is_empty() {
        return Err("operation_id must be non-empty".into());
    }
    Ok(v7_from_hash(s.as_bytes()))
}

/// Cluster id from daemon string (hyphenated UUIDv7 or stable hash).
pub fn parse_cluster_id(s: &str) -> ClusterId {
    s.parse()
        .unwrap_or_else(|_| ClusterId::from_bytes(v7_from_hash(s.as_bytes())).expect("v7 hash"))
}

/// Capsule id derived from content digest (stable for the same bytes).
pub fn capsule_id_for_digest(digest: &[u8; 32]) -> CapsuleId {
    CapsuleId::from_bytes(v7_from_hash(digest)).expect("v7 hash")
}

fn v7_from_hash(bytes: &[u8]) -> [u8; 16] {
    let h = blake3::hash(bytes);
    let mut b = [0u8; 16];
    b.copy_from_slice(&h.as_bytes()[..16]);
    b[6] = (b[6] & 0x0f) | 0x70;
    b[8] = (b[8] & 0x3f) | 0x80;
    b
}

fn parse_hex_bytes(hex: &str) -> Result<Vec<u8>, String> {
    if hex.len() % 2 != 0 {
        return Err("capsule hex length must be even".into());
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).map_err(|_| "invalid capsule hex utf8".to_string())?;
        out.push(u8::from_str_radix(s, 16).map_err(|_| format!("bad capsule hex at byte {i}"))?);
    }
    Ok(out)
}

/// Decode optional capsule hex and verify BLAKE3 matches `content_digest`.
pub fn decode_capsule_hex(
    hex: Option<&str>,
    content_digest: &[u8; 32],
) -> Result<Option<Vec<u8>>, String> {
    let Some(hex) = hex.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let bytes = parse_hex_bytes(hex)?;
    let got = *blake3::hash(&bytes).as_bytes();
    if &got != content_digest {
        return Err("capsule bytes digest mismatch vs --digest / content_digest_hex".into());
    }
    Ok(Some(bytes))
}

/// Read framing identity before assigning an object key. This does not grant
/// trust; the product deploy path verifies the signature and enrollment while
/// the object is quarantined.
pub fn capsule_identity_reader(
    reader: &mut dyn std::io::Read,
) -> Result<(ClusterId, CapsuleId), String> {
    let meta = StreamingCapsuleReader::new(reader)
        .verify()
        .map_err(|e| format!("invalid Capsule framing: {e}"))?;
    let cluster = ClusterId::from_bytes(meta.header.cluster_id)
        .map_err(|_| "Capsule cluster_id is not UUIDv7".to_string())?;
    let capsule = CapsuleId::from_bytes(meta.header.capsule_id)
        .map_err(|_| "Capsule capsule_id is not UUIDv7".to_string())?;
    Ok((cluster, capsule))
}

pub fn capsule_identity(bytes: &[u8]) -> Result<(ClusterId, CapsuleId), String> {
    capsule_identity_reader(&mut Cursor::new(bytes))
}

fn object_uri(key: &str) -> String {
    format!("fake-object://{key}")
}

fn publish_capsule<S: ObjectStore>(
    store: &mut S,
    req: &DeployTxnRequest,
) -> Result<ObjectLocator, DeployTxnOutcome> {
    let final_key = final_capsule_key(req.cluster_id, req.capsule_id).map_err(|e| {
        DeployTxnOutcome::Failed {
            phase: DeployPhase::LocalValidation,
            reason: e.to_string(),
            orphan: None,
        }
    })?;

    if let Ok(ev) = store.head(&final_key) {
        if ev.digest == req.content_digest {
            return Ok(ObjectLocator {
                key: final_key.clone(),
                uri: object_uri(final_key.as_str()),
            });
        }
        return Err(DeployTxnOutcome::Failed {
            phase: DeployPhase::ImmutablePublish,
            reason: "final Capsule key exists with divergent digest".into(),
            orphan: None,
        });
    }

    let Some(bytes) = req.capsule_bytes.as_ref() else {
        return Err(DeployTxnOutcome::Failed {
            phase: DeployPhase::LocalValidation,
            reason: "capsule bytes required when object is not already published (direct uploads do not execute)".into(),
            orphan: None,
        });
    };

    let upload = store
        .begin_quarantine(req.cluster_id, req.capsule_id, bytes.len() as u64)
        .map_err(|e| DeployTxnOutcome::Failed {
            phase: DeployPhase::CapsulePersist,
            reason: e.to_string(),
            orphan: None,
        })?;
    if let Err(e) = store.write(upload, bytes) {
        let _ = store.abort(upload);
        return Err(DeployTxnOutcome::Failed {
            phase: DeployPhase::CapsulePersist,
            reason: e.to_string(),
            orphan: None,
        });
    }
    let quarantine = match store.finish_quarantine(upload, req.content_digest) {
        Ok(ev) => ev.key,
        Err(e) => {
            return Err(DeployTxnOutcome::Failed {
                phase: DeployPhase::CapsulePersist,
                reason: e.to_string(),
                orphan: None,
            });
        }
    };
    match store.publish_if_absent(
        &quarantine,
        &final_key,
        req.content_digest,
        bytes.len() as u64,
    ) {
        Ok(_) => Ok(ObjectLocator {
            key: final_key.clone(),
            uri: object_uri(final_key.as_str()),
        }),
        Err(e) if e.kind() == ObjectStoreErrorKind::Conflict => {
            // Dest occupied: succeed only when head matches our digest (idempotent).
            match store.head(&final_key) {
                Ok(ev) if ev.digest == req.content_digest => Ok(ObjectLocator {
                    key: final_key.clone(),
                    uri: object_uri(final_key.as_str()),
                }),
                _ => Err(DeployTxnOutcome::Failed {
                    phase: DeployPhase::ImmutablePublish,
                    reason: e.to_string(),
                    orphan: None,
                }),
            }
        }
        Err(e) => Err(DeployTxnOutcome::Failed {
            phase: DeployPhase::ImmutablePublish,
            reason: e.to_string(),
            orphan: None,
        }),
    }
}

/// Run upload → publish → Raft-idempotent intent accept (wait=`accepted`).
///
/// Execution/placement beyond intent acceptance is GUMP-N011/N012. Orphans are
/// appended when publish succeeds but intent accept fails.
pub fn run_deploy_txn<S: ObjectStore>(
    store: &mut S,
    cluster: &MemoryCluster,
    orphans: &mut Vec<OrphanCapsule>,
    req: DeployTxnRequest,
) -> DeployTxnOutcome {
    let request_digest = deploy_request_digest(&req);
    let object = match publish_capsule(store, &req) {
        Ok(o) => o,
        Err(outcome) => return outcome,
    };

    let payload = match serde_json::to_vec(&DesiredCapsuleBindingV1 {
        schema: "gump.desired-capsule/1".into(),
        operation_id: req.operation_id_display.clone(),
        capsule_id: req.capsule_id.to_hyphenated(),
    }) {
        Ok(payload) => payload,
        Err(e) => {
            return DeployTxnOutcome::Failed {
                phase: DeployPhase::LocalValidation,
                reason: format!("encode desired Capsule binding: {e}"),
                orphan: None,
            };
        }
    };
    let cmd = RaftCommand::Idempotent {
        operation_id: req.operation_id,
        request_digest,
        inner: Box::new(RaftCommand::PutDesired {
            namespace: req.namespace.clone(),
            app: req.app.clone(),
            expected_generation: req.expected_generation,
            payload,
            content_digest: req.content_digest,
        }),
    };

    match cluster.client_write(cmd) {
        Ok(RaftResponse::Applied(o)) => DeployTxnOutcome::Success {
            desired_generation: o.desired_generation.unwrap_or(1),
            cluster_revision: o.revision,
            object,
            replayed: false,
        },
        Ok(RaftResponse::Replay(inner)) => match *inner {
            RaftResponse::Applied(o) => DeployTxnOutcome::Success {
                desired_generation: o.desired_generation.unwrap_or(1),
                cluster_revision: o.revision,
                object,
                replayed: true,
            },
            other => DeployTxnOutcome::Failed {
                phase: DeployPhase::IntentAccept,
                reason: format!("unexpected replay payload {other:?}"),
                orphan: None,
            },
        },
        Ok(RaftResponse::Rejected(msg)) => {
            if msg.contains("idempotency conflict") {
                return DeployTxnOutcome::Conflict {
                    operation_id: req.operation_id,
                };
            }
            let orphan = OrphanCapsule {
                capsule_id: req.capsule_id,
                capsule_digest: req.content_digest,
                object: object.clone(),
                operation_id: req.operation_id,
            };
            orphans.push(orphan.clone());
            DeployTxnOutcome::Failed {
                phase: DeployPhase::IntentAccept,
                reason: msg,
                orphan: Some(orphan),
            }
        }
        Err(e) => {
            let orphan = OrphanCapsule {
                capsule_id: req.capsule_id,
                capsule_digest: req.content_digest,
                object: object.clone(),
                operation_id: req.operation_id,
            };
            orphans.push(orphan.clone());
            DeployTxnOutcome::Failed {
                phase: DeployPhase::IntentAccept,
                reason: e,
                orphan: Some(orphan),
            }
        }
    }
}

/// Product deploy path: quarantine, stream-verify the complete Capsule,
/// authorize its signer, immutably publish, then commit intent.
pub fn run_verified_deploy_txn<S: ObjectStore>(
    store: &mut S,
    trust: &SignerTrustPolicy,
    cluster: &MemoryCluster,
    orphans: &mut Vec<OrphanCapsule>,
    mut req: DeployTxnRequest,
    now_ms: u64,
) -> DeployTxnOutcome {
    let Some(bytes) = req.capsule_bytes.take() else {
        return DeployTxnOutcome::Failed {
            phase: DeployPhase::LocalValidation,
            reason: "verified deploy requires the exact sealed Capsule body".into(),
            orphan: None,
        };
    };
    let mut reader = Cursor::new(bytes.as_slice());
    run_verified_deploy_reader(
        store,
        trust,
        cluster,
        orphans,
        req,
        now_ms,
        bytes.len() as u64,
        &mut reader,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_verified_deploy_reader<S: ObjectStore>(
    store: &mut S,
    trust: &SignerTrustPolicy,
    cluster: &MemoryCluster,
    orphans: &mut Vec<OrphanCapsule>,
    req: DeployTxnRequest,
    now_ms: u64,
    content_length: u64,
    reader: &mut dyn std::io::Read,
) -> DeployTxnOutcome {
    let receipt = match StreamedIngress::default().accept_known_length(
        store,
        trust,
        req.cluster_id,
        req.capsule_id,
        &req.namespace,
        now_ms,
        content_length,
        reader,
    ) {
        Ok(r) => r,
        Err(e) => {
            return DeployTxnOutcome::Failed {
                phase: DeployPhase::Authz,
                reason: e.to_string(),
                orphan: None,
            };
        }
    };
    if receipt.stats.digest != req.content_digest {
        return DeployTxnOutcome::Failed {
            phase: DeployPhase::LocalValidation,
            reason: "verified Capsule digest differs from declared content digest".into(),
            orphan: None,
        };
    }
    if let Err(reason) = verify_requested_application(store, &req) {
        return DeployTxnOutcome::Failed {
            phase: DeployPhase::LocalValidation,
            reason,
            orphan: None,
        };
    }
    run_deploy_txn(store, cluster, orphans, req)
}

fn verify_requested_application<S: ObjectStore>(
    store: &S,
    req: &DeployTxnRequest,
) -> Result<(), String> {
    let key = final_capsule_key(req.cluster_id, req.capsule_id).map_err(|e| e.to_string())?;
    let meta = StreamingCapsuleReader::new(
        store
            .get_reader(&key, None)
            .map_err(|e| format!("open verified Capsule metadata: {e}"))?,
    )
    .verify()
    .map_err(|e| format!("verify published Capsule metadata: {e}"))?;
    let descriptor = meta
        .table
        .descriptors
        .iter()
        .find(|descriptor| descriptor.segment_type == SegmentType::PublicMetadata)
        .ok_or("Capsule lacks public release metadata")?;
    let start = meta.inner_file_offset.saturating_add(descriptor.offset);
    let public = store
        .get(
            &key,
            Some(ByteRange {
                start,
                end: Some(start.saturating_add(descriptor.stored_length)),
            }),
        )
        .map_err(|e| format!("read verified Capsule metadata: {e}"))?;
    let release = ReleaseMetadataV1::decode(public.as_slice())
        .map_err(|e| format!("decode verified release metadata: {e}"))?;
    let app = release
        .normalized_manifest
        .and_then(|manifest| manifest.app)
        .or(release.app)
        .ok_or("Capsule lacks signed application identity")?;
    if app.namespace != req.namespace || app.app_id != req.app {
        return Err(format!(
            "deployment target {}/{} does not match signed Capsule application {}/{}",
            req.namespace, req.app, app.namespace, app.app_id
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gump_connectors::FakeObjectStore;
    use std::sync::Arc;

    fn digest(bytes: &[u8]) -> [u8; 32] {
        *blake3::hash(bytes).as_bytes()
    }

    fn cluster() -> Arc<MemoryCluster> {
        Arc::new(MemoryCluster::bootstrap_one_voter(1, 1).expect("raft"))
    }

    fn base_req(bytes: &[u8]) -> DeployTxnRequest {
        let content_digest = digest(bytes);
        let op = parse_operation_id("019fdad7-510c-7ef0-8a2f-8ee3db130710").unwrap();
        DeployTxnRequest {
            operation_id: op,
            operation_id_display: "019fdad7-510c-7ef0-8a2f-8ee3db130710".into(),
            namespace: "default".into(),
            app: "demo".into(),
            expected_generation: 0,
            content_digest,
            capsule_bytes: Some(bytes.to_vec()),
            cluster_id: ClusterId::from_bytes(v7_from_hash(b"test-cluster")).unwrap(),
            capsule_id: capsule_id_for_digest(&content_digest),
        }
    }

    #[test]
    fn upload_intent_replay_and_conflict() {
        let mut store = FakeObjectStore::new();
        let cluster = cluster();
        let mut orphans = Vec::new();
        let body = b"sealed-capsule-bytes-n010";
        let req = base_req(body);

        let first = run_deploy_txn(&mut store, &cluster, &mut orphans, req.clone());
        match &first {
            DeployTxnOutcome::Success {
                desired_generation,
                replayed,
                ..
            } => {
                assert_eq!(*desired_generation, 1);
                assert!(!*replayed);
            }
            other => panic!("expected success, got {other:?}"),
        }
        assert!(orphans.is_empty());

        let replay = run_deploy_txn(&mut store, &cluster, &mut orphans, req.clone());
        assert!(matches!(
            replay,
            DeployTxnOutcome::Success { replayed: true, .. }
        ));

        let mut conflict_req = req;
        conflict_req.content_digest = digest(b"different-content");
        conflict_req.capsule_bytes = Some(b"different-content".to_vec());
        conflict_req.capsule_id = capsule_id_for_digest(&conflict_req.content_digest);
        let conflict = run_deploy_txn(&mut store, &cluster, &mut orphans, conflict_req);
        assert!(matches!(conflict, DeployTxnOutcome::Conflict { .. }));
    }

    #[test]
    fn publish_without_intent_reports_orphan() {
        let mut store = FakeObjectStore::new();
        let cluster = cluster();
        let mut orphans = Vec::new();

        // Occupy generation 0→1 under the same (ns, app) so the deploy's
        // expected_generation=0 PutDesired is rejected after publish.
        let seed = cluster
            .client_write(RaftCommand::PutDesired {
                namespace: "default".into(),
                app: "demo".into(),
                expected_generation: 0,
                payload: b"seed".to_vec(),
                content_digest: digest(b"seed"),
            })
            .expect("seed write");
        assert!(matches!(seed, RaftResponse::Applied(_)));

        let body = b"orphan-capsule";
        let outcome = run_deploy_txn(&mut store, &cluster, &mut orphans, base_req(body));
        match outcome {
            DeployTxnOutcome::Failed {
                phase,
                orphan: Some(o),
                ..
            } => {
                assert_eq!(phase, DeployPhase::IntentAccept);
                assert_eq!(o.capsule_digest, digest(body));
            }
            other => panic!("expected orphan failure, got {other:?}"),
        }
        assert_eq!(orphans.len(), 1);
        // Object remains published (inert) — head succeeds.
        let key = final_capsule_key(
            ClusterId::from_bytes(v7_from_hash(b"test-cluster")).unwrap(),
            capsule_id_for_digest(&digest(body)),
        )
        .unwrap();
        assert_eq!(store.head(&key).unwrap().digest, digest(body));
    }

    #[test]
    fn later_deploy_advances_generation_with_compare_and_set() {
        let mut store = FakeObjectStore::new();
        let cluster = cluster();
        let mut orphans = Vec::new();
        let first = base_req(b"release-one");
        assert!(matches!(
            run_deploy_txn(&mut store, &cluster, &mut orphans, first),
            DeployTxnOutcome::Success {
                desired_generation: 1,
                ..
            }
        ));

        let mut second = base_req(b"release-two");
        second.operation_id = parse_operation_id("019fdad7-510c-7ef0-8a2f-8ee3db130711").unwrap();
        second.operation_id_display = "019fdad7-510c-7ef0-8a2f-8ee3db130711".into();
        second.expected_generation = 1;
        assert!(matches!(
            run_deploy_txn(&mut store, &cluster, &mut orphans, second),
            DeployTxnOutcome::Success {
                desired_generation: 2,
                ..
            }
        ));
        assert!(orphans.is_empty());
    }

    #[test]
    fn digest_only_without_published_object_fails_closed() {
        let mut store = FakeObjectStore::new();
        let cluster = cluster();
        let mut orphans = Vec::new();
        let mut req = base_req(b"x");
        req.capsule_bytes = None;
        let outcome = run_deploy_txn(&mut store, &cluster, &mut orphans, req);
        assert!(matches!(
            outcome,
            DeployTxnOutcome::Failed {
                phase: DeployPhase::LocalValidation,
                ..
            }
        ));
        assert!(orphans.is_empty());
    }
}
