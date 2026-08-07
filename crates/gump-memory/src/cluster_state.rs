//! Single replicated cluster state applied through OpenRaft (STL-01 / D006).
//!
//! Authoritative mutations enter only as [`RaftCommand`] entries. OpenRaft's
//! `StoredMembership` is the sole voter/learner authority; application member
//! phase tracking must not decide commit quorums.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::authority::{ControllerAuthority, FenceToken};
use crate::records::{ApplyError, ApplyResult, Command, TypedRecordMachine};

/// Serializable OpenRaft application request (replaces placeholder ClientRequest).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RaftCommand {
    /// Typed K/V / lease / txn mutation (PROTOCOL.md §6–§8).
    Record(Command),
    /// Acquire controller fence; lease lives in the record machine lease table.
    AcquireController { holder: u64 },
    /// Desired workload declaration bytes (connectors validate, then submit).
    PutDesired {
        namespace: String,
        app: String,
        /// Expected current generation before accept (`0` = create).
        expected_generation: u64,
        payload: Vec<u8>,
        content_digest: [u8; 32],
    },
    /// Replay-safe envelope: same `operation_id` + digest returns the prior response.
    Idempotent {
        operation_id: [u8; 16],
        request_digest: [u8; 32],
        inner: Box<RaftCommand>,
    },
}

/// Deterministic OpenRaft application response.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RaftResponse {
    Applied(ApplyOutcome),
    /// Idempotent replay of a prior applied response.
    Replay(Box<RaftResponse>),
    Rejected(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApplyOutcome {
    pub revision: u64,
    pub txn_succeeded: Option<bool>,
    pub lease_id: Option<u64>,
    pub expired_lease_ids: Vec<u64>,
    pub controller: Option<FenceToken>,
    pub desired_generation: Option<u64>,
}

impl From<ApplyResult> for ApplyOutcome {
    fn from(r: ApplyResult) -> Self {
        Self {
            revision: r.revision,
            txn_succeeded: r.txn_succeeded,
            lease_id: r.lease.map(|l| l.id),
            expired_lease_ids: r.expired_lease_ids,
            controller: None,
            desired_generation: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct DesiredEntry {
    generation: u64,
    content_digest: [u8; 32],
    payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct IdempotencyEntry {
    request_digest: [u8; 32],
    response: RaftResponse,
}

/// One cluster's replicated application state (RAM-only; snapshotted in-memory).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ClusterState {
    records: TypedRecordMachine,
    controller: ControllerAuthority,
    /// `(namespace, app)` → accepted desired declaration (opaque validated bytes).
    desired: BTreeMap<(String, String), DesiredEntry>,
    /// Operation receipts retained with the mutation that produced them.
    idempotency: BTreeMap<[u8; 16], IdempotencyEntry>,
}

impl ClusterState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn records(&self) -> &TypedRecordMachine {
        &self.records
    }

    pub fn records_mut(&mut self) -> &mut TypedRecordMachine {
        &mut self.records
    }

    pub fn controller(&self) -> &ControllerAuthority {
        &self.controller
    }

    pub fn desired_generation(&self, namespace: &str, app: &str) -> Option<u64> {
        self.desired
            .get(&(namespace.to_string(), app.to_string()))
            .map(|e| e.generation)
    }

    pub fn apply(&mut self, cmd: RaftCommand) -> RaftResponse {
        match cmd {
            RaftCommand::Idempotent {
                operation_id,
                request_digest,
                inner,
            } => self.apply_idempotent(operation_id, request_digest, *inner),
            other => self.apply_inner(other),
        }
    }

    fn apply_idempotent(
        &mut self,
        operation_id: [u8; 16],
        request_digest: [u8; 32],
        inner: RaftCommand,
    ) -> RaftResponse {
        if let Some(prev) = self.idempotency.get(&operation_id) {
            if prev.request_digest == request_digest {
                return RaftResponse::Replay(Box::new(prev.response.clone()));
            }
            return RaftResponse::Rejected(format!(
                "idempotency conflict for operation {}",
                hex16(&operation_id)
            ));
        }
        // Nested Idempotent is rejected to keep apply deterministic and shallow.
        if matches!(inner, RaftCommand::Idempotent { .. }) {
            return RaftResponse::Rejected("nested Idempotent commands are not allowed".into());
        }
        let response = self.apply_inner(inner);
        if matches!(response, RaftResponse::Applied(_)) {
            self.idempotency.insert(
                operation_id,
                IdempotencyEntry {
                    request_digest,
                    response: response.clone(),
                },
            );
        }
        response
    }

    fn apply_inner(&mut self, cmd: RaftCommand) -> RaftResponse {
        match cmd {
            RaftCommand::Record(record_cmd) => match self.records.apply(record_cmd) {
                Ok(r) => RaftResponse::Applied(r.into()),
                Err(e) => RaftResponse::Rejected(format_apply_error(&e)),
            },
            RaftCommand::AcquireController { holder } => {
                let now = self.records.now_ms();
                let token = self
                    .controller
                    .acquire(holder, now, self.records.leases_mut());
                let revision = self.records.bump_revision();
                RaftResponse::Applied(ApplyOutcome {
                    revision,
                    txn_succeeded: None,
                    lease_id: Some(token.lease_id),
                    expired_lease_ids: Vec::new(),
                    controller: Some(token),
                    desired_generation: None,
                })
            }
            RaftCommand::PutDesired {
                namespace,
                app,
                expected_generation,
                payload,
                content_digest,
            } => {
                let key = (namespace, app);
                let current = self.desired.get(&key).map(|e| e.generation).unwrap_or(0);
                if current != expected_generation {
                    return RaftResponse::Rejected(format!(
                        "generation conflict: current={current} expected={expected_generation}"
                    ));
                }
                let next_gen = expected_generation.saturating_add(1);
                if let Some(prev) = self.desired.get(&key) {
                    if prev.generation == next_gen && prev.content_digest != content_digest {
                        return RaftResponse::Rejected(format!(
                            "divergent content at generation {next_gen}"
                        ));
                    }
                    if prev.generation == next_gen && prev.content_digest == content_digest {
                        return RaftResponse::Applied(ApplyOutcome {
                            revision: self.records.revision(),
                            txn_succeeded: None,
                            lease_id: None,
                            expired_lease_ids: Vec::new(),
                            controller: None,
                            desired_generation: Some(next_gen),
                        });
                    }
                }
                self.desired.insert(
                    key,
                    DesiredEntry {
                        generation: next_gen,
                        content_digest,
                        payload,
                    },
                );
                let revision = self.records.bump_revision();
                RaftResponse::Applied(ApplyOutcome {
                    revision,
                    txn_succeeded: None,
                    lease_id: None,
                    expired_lease_ids: Vec::new(),
                    controller: None,
                    desired_generation: Some(next_gen),
                })
            }
            RaftCommand::Idempotent { .. } => {
                RaftResponse::Rejected("nested Idempotent must use apply() entrypoint".into())
            }
        }
    }
}

fn format_apply_error(e: &ApplyError) -> String {
    e.to_string()
}

fn hex16(bytes: &[u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
