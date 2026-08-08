//! Explicit full-loss recovery (GUMP-N016 / DELIVERY D06).
//!
//! Capsules in the object store stay inert until an authorized actor selects
//! them. Inventory/inspect never create desired state. Reintroduce creates
//! **fresh** intent only — never restores assumed prior completion or placement.

#![allow(clippy::result_large_err)]

use gump_connectors::{FakeObjectStore, ObjectStore, ObjectStoreErrorKind, final_capsule_key};
use gump_memory::{MemoryCluster, RaftCommand, RaftResponse};
use gump_types::CapsuleId;

use crate::deploy_txn::{parse_cluster_id, parse_operation_id};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedCapsule {
    pub capsule_id: CapsuleId,
    pub content_digest: [u8; 32],
    pub size_bytes: u64,
    pub object_key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReintroduceOutcome {
    Plan {
        capsule: VerifiedCapsule,
        finite_mode: String,
    },
    Applied {
        capsule: VerifiedCapsule,
        finite_mode: String,
        desired_generation: u64,
        replayed: bool,
    },
    Failed {
        reason: String,
        code: &'static str,
    },
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Normalize finite-work decision. Required for non-plan reintroduce (INV-018).
pub fn normalize_finite_mode(
    plan: bool,
    finite_mode: Option<&str>,
    resume_from: Option<&str>,
) -> Result<Option<String>, String> {
    let mode = finite_mode.map(str::trim).filter(|s| !s.is_empty());
    match mode {
        None if plan => Ok(None),
        None => Err(
            "reintroduce requires --new-execution or --resume-from <checkpoint> (finite work is never inferred)"
                .into(),
        ),
        Some("new_execution") | Some("new-execution") => {
            if resume_from.map(str::trim).filter(|s| !s.is_empty()).is_some() {
                return Err("--resume-from is incompatible with --new-execution".into());
            }
            Ok(Some("new_execution".into()))
        }
        Some("resume") => {
            let Some(r) = resume_from.map(str::trim).filter(|s| !s.is_empty()) else {
                return Err("--resume-from is required when finite_mode=resume".into());
            };
            if r.len() > 512 {
                return Err("resume-from reference too long".into());
            }
            Ok(Some(format!("resume:{r}")))
        }
        Some(other) => Err(format!(
            "unknown finite_mode {other:?}; use new_execution or resume"
        )),
    }
}

pub fn verify_stored_capsule(
    store: &FakeObjectStore,
    cluster_id: &str,
    capsule_id: &str,
) -> Result<VerifiedCapsule, String> {
    let capsule: CapsuleId = capsule_id
        .parse()
        .map_err(|_| format!("invalid capsule_id {capsule_id:?}"))?;
    let cluster = parse_cluster_id(cluster_id);
    let key = final_capsule_key(cluster, capsule).map_err(|e| e.to_string())?;
    let ev = store.head(&key).map_err(|e| match e.kind() {
        ObjectStoreErrorKind::NotFound => {
            format!("Capsule {capsule_id} not found in object store (remains inert)")
        }
        _ => format!("object store head failed: {e}"),
    })?;
    Ok(VerifiedCapsule {
        capsule_id: capsule,
        content_digest: ev.digest,
        size_bytes: ev.length,
        object_key: ev.key.as_str().to_string(),
    })
}

pub fn digest_hex(digest: &[u8; 32]) -> String {
    to_hex(digest)
}

fn reintroduce_request_digest(
    operation_id: &[u8; 16],
    content_digest: &[u8; 32],
    namespace: &str,
    app: &str,
    finite_mode: &str,
) -> [u8; 32] {
    let mut buf = Vec::with_capacity(16 + 32 + namespace.len() + app.len() + finite_mode.len() + 4);
    buf.extend_from_slice(operation_id);
    buf.extend_from_slice(content_digest);
    buf.extend_from_slice(namespace.as_bytes());
    buf.push(0);
    buf.extend_from_slice(app.as_bytes());
    buf.push(0);
    buf.extend_from_slice(b"reintroduce");
    buf.push(0);
    buf.extend_from_slice(finite_mode.as_bytes());
    *blake3::hash(&buf).as_bytes()
}

/// Inputs for plan/apply reintroduce (GUMP-N016).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReintroduceRequest<'a> {
    pub cluster_id: &'a str,
    pub capsule_id: &'a str,
    pub plan: bool,
    pub finite_mode: Option<&'a str>,
    pub resume_from: Option<&'a str>,
    pub operation_id: Option<&'a str>,
    pub namespace: &'a str,
    pub app: &'a str,
}

/// Plan or apply fresh intent for one selected Capsule already in the store.
pub fn run_reintroduce(
    store: &FakeObjectStore,
    cluster: &MemoryCluster,
    req: &ReintroduceRequest<'_>,
) -> ReintroduceOutcome {
    let finite = match normalize_finite_mode(req.plan, req.finite_mode, req.resume_from) {
        Ok(f) => f,
        Err(reason) => {
            return ReintroduceOutcome::Failed {
                reason,
                code: "INVALID_ARGUMENT",
            };
        }
    };

    let capsule = match verify_stored_capsule(store, req.cluster_id, req.capsule_id) {
        Ok(c) => c,
        Err(reason) => {
            return ReintroduceOutcome::Failed {
                reason,
                code: "NOT_FOUND",
            };
        }
    };

    let mode_label = finite.clone().unwrap_or_else(|| "unspecified_plan".into());

    if req.plan {
        return ReintroduceOutcome::Plan {
            capsule,
            finite_mode: mode_label,
        };
    }

    let finite_mode = finite.expect("non-plan requires finite_mode");
    let op_display = req
        .operation_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("reintroduce-{}", capsule.capsule_id.to_hyphenated()));
    let op_bytes = match parse_operation_id(&op_display) {
        Ok(b) => b,
        Err(reason) => {
            return ReintroduceOutcome::Failed {
                reason,
                code: "INVALID_ARGUMENT",
            };
        }
    };

    // Payload is operator-visible intent only — never Capsule ciphertext/secrets.
    let payload = format!("reintroduce:{finite_mode}:{op_display}").into_bytes();
    let request_digest = reintroduce_request_digest(
        &op_bytes,
        &capsule.content_digest,
        req.namespace,
        req.app,
        &finite_mode,
    );
    let cmd = RaftCommand::Idempotent {
        operation_id: op_bytes,
        request_digest,
        inner: Box::new(RaftCommand::PutDesired {
            namespace: req.namespace.to_string(),
            app: req.app.to_string(),
            expected_generation: 0,
            payload,
            content_digest: capsule.content_digest,
        }),
    };

    match cluster.client_write(cmd) {
        Ok(RaftResponse::Applied(o)) => ReintroduceOutcome::Applied {
            capsule,
            finite_mode,
            desired_generation: o.desired_generation.unwrap_or(1),
            replayed: false,
        },
        Ok(RaftResponse::Replay(inner)) => match *inner {
            RaftResponse::Applied(o) => ReintroduceOutcome::Applied {
                capsule,
                finite_mode,
                desired_generation: o.desired_generation.unwrap_or(1),
                replayed: true,
            },
            other => ReintroduceOutcome::Failed {
                reason: format!("unexpected replay payload {other:?}"),
                code: "INTERNAL",
            },
        },
        Ok(RaftResponse::Rejected(msg)) => {
            if msg.contains("idempotency conflict") {
                ReintroduceOutcome::Failed {
                    reason: "same operation_id with a different reintroduce request".into(),
                    code: "CONFLICT",
                }
            } else {
                // Capsule stays inert — no desired state accepted.
                ReintroduceOutcome::Failed {
                    reason: msg,
                    code: "FAILED_PRECONDITION",
                }
            }
        }
        Err(e) => ReintroduceOutcome::Failed {
            reason: e,
            code: "UNAVAILABLE",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy_txn::capsule_id_for_digest;
    use gump_connectors::{ObjectStore, final_capsule_key};
    use gump_types::CapsuleId;
    use std::sync::Arc;

    fn publish(store: &mut FakeObjectStore, body: &[u8]) -> (CapsuleId, [u8; 32]) {
        let digest = *blake3::hash(body).as_bytes();
        let capsule = capsule_id_for_digest(&digest);
        let cluster = parse_cluster_id("n016-cluster");
        let upload = store
            .begin_quarantine(cluster, capsule, body.len() as u64)
            .unwrap();
        store.write(upload, body).unwrap();
        let q = store.finish_quarantine(upload, digest).unwrap().key;
        let final_key = final_capsule_key(cluster, capsule).unwrap();
        store
            .publish_if_absent(&q, &final_key, digest, body.len() as u64)
            .unwrap();
        (capsule, digest)
    }

    #[test]
    fn plan_does_not_mutate_desired() {
        let mut store = FakeObjectStore::new();
        let (capsule, _) = publish(&mut store, b"n016-plan");
        let cluster = Arc::new(MemoryCluster::bootstrap_one_voter(1, 1).unwrap());
        let cluster_id = parse_cluster_id("n016-cluster").to_hyphenated();
        let out = run_reintroduce(
            &store,
            &cluster,
            &ReintroduceRequest {
                cluster_id: &cluster_id,
                capsule_id: &capsule.to_hyphenated(),
                plan: true,
                finite_mode: Some("new_execution"),
                resume_from: None,
                operation_id: None,
                namespace: "default",
                app: "demo",
            },
        );
        assert!(matches!(out, ReintroduceOutcome::Plan { .. }));
        assert_eq!(cluster.desired_len().unwrap(), 0);
    }
}
