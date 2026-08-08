//! Single replicated cluster state applied through OpenRaft (STL-01 / D006).
//!
//! Authoritative mutations enter only as [`RaftCommand`] entries. OpenRaft's
//! `StoredMembership` is the sole voter/learner authority; application member
//! phase tracking must not decide commit quorums.
//!
//! Idempotency receipts are bounded (D014 / STL-15): 24h TTL + 100_000 ceiling,
//! keyed to the committed record-machine clock. External crates mutate state
//! only through [`ClusterState::apply`] — there is no public `records_mut`.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};

use crate::authority::{ControllerAuthority, FenceToken};
use crate::records::{ApplyError, ApplyResult, Command, KeyPrefix, TypedRecordMachine};
use gump_types::Label;

/// D014 / PROTOCOL §15: max retained operation receipts in authoritative memory.
pub const IDEMPOTENCY_MAX_ENTRIES: usize = 100_000;
/// D014: receipt TTL measured against committed cluster time (`TypedRecordMachine::now_ms`).
pub const IDEMPOTENCY_TTL_MS: u64 = 24 * 60 * 60 * 1_000;

/// PROTOCOL §7 `/workloads/.../desired` payload ceiling (also `KeyPrefix::WorkloadsDesired`).
pub const DESIRED_MAX_PAYLOAD_BYTES: usize = 256 * 1024;
/// Bound on distinct `(namespace, app)` desired entries (STL-23 / RAM=DB).
pub const DESIRED_MAX_ENTRIES: usize = 8_192;
/// Bound on total retained desired bytes (namespace+app+payload) across the map.
pub const DESIRED_MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;

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
    /// Committed clock when the Applied response was recorded (STL-02 time).
    #[serde(default)]
    recorded_at_ms: u64,
}

/// One cluster's replicated application state (RAM-only; snapshotted in-memory).
#[derive(Clone, Debug, Serialize)]
pub struct ClusterState {
    records: TypedRecordMachine,
    controller: ControllerAuthority,
    /// `(namespace, app)` → accepted desired declaration (opaque validated bytes).
    desired: BTreeMap<(String, String), DesiredEntry>,
    /// Operation receipts retained with the mutation that produced them.
    idempotency: BTreeMap<[u8; 16], IdempotencyEntry>,
    /// Receipt ceiling (D014). Not snapshotted — restored to production defaults.
    #[serde(skip)]
    idempotency_max_entries: usize,
    /// Receipt TTL in committed-time ms (D014).
    #[serde(skip)]
    idempotency_ttl_ms: u64,
    /// Min `recorded_at_ms` among receipts — skips full TTL scans when nothing can expire.
    /// Recomputed on snapshot deserialize (STL-23); never trusted from wire alone.
    #[serde(skip)]
    idempotency_oldest_ms: Option<u64>,
    #[serde(skip)]
    desired_max_entries: usize,
    #[serde(skip)]
    desired_max_total_bytes: usize,
}

#[derive(Deserialize)]
struct ClusterStateWire {
    records: TypedRecordMachine,
    controller: ControllerAuthority,
    desired: BTreeMap<(String, String), DesiredEntry>,
    idempotency: BTreeMap<[u8; 16], IdempotencyEntry>,
}

impl<'de> Deserialize<'de> for ClusterState {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = ClusterStateWire::deserialize(deserializer)?;
        let mut state = ClusterState {
            records: wire.records,
            controller: wire.controller,
            desired: wire.desired,
            idempotency: wire.idempotency,
            idempotency_max_entries: IDEMPOTENCY_MAX_ENTRIES,
            idempotency_ttl_ms: IDEMPOTENCY_TTL_MS,
            idempotency_oldest_ms: None,
            desired_max_entries: DESIRED_MAX_ENTRIES,
            desired_max_total_bytes: DESIRED_MAX_TOTAL_BYTES,
        };
        // STL-23: restore TTL index from receipt bodies (field is #[serde(skip)]).
        state.recompute_idempotency_oldest();
        Ok(state)
    }
}

impl Default for ClusterState {
    fn default() -> Self {
        Self {
            records: TypedRecordMachine::default(),
            controller: ControllerAuthority::default(),
            desired: BTreeMap::new(),
            idempotency: BTreeMap::new(),
            idempotency_max_entries: IDEMPOTENCY_MAX_ENTRIES,
            idempotency_ttl_ms: IDEMPOTENCY_TTL_MS,
            idempotency_oldest_ms: None,
            desired_max_entries: DESIRED_MAX_ENTRIES,
            desired_max_total_bytes: DESIRED_MAX_TOTAL_BYTES,
        }
    }
}

impl ClusterState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test/harness constructor with custom receipt bounds (production uses D014 defaults).
    pub fn with_idempotency_limits(max_entries: usize, ttl_ms: u64) -> Self {
        Self {
            idempotency_max_entries: max_entries.max(1),
            idempotency_ttl_ms: ttl_ms,
            ..Self::default()
        }
    }

    /// Test/harness constructor with custom desired-state bounds (STL-23).
    pub fn with_desired_limits(max_entries: usize, max_total_bytes: usize) -> Self {
        Self {
            desired_max_entries: max_entries.max(1),
            desired_max_total_bytes: max_total_bytes.max(1),
            ..Self::default()
        }
    }

    pub fn records(&self) -> &TypedRecordMachine {
        &self.records
    }

    pub fn controller(&self) -> &ControllerAuthority {
        &self.controller
    }

    pub fn desired_generation(&self, namespace: &str, app: &str) -> Option<u64> {
        self.desired
            .get(&(namespace.to_string(), app.to_string()))
            .map(|e| e.generation)
    }

    /// Number of retained idempotency receipts (test/ops introspection).
    pub fn idempotency_len(&self) -> usize {
        self.idempotency.len()
    }

    /// Number of retained desired declarations (test/ops introspection).
    pub fn desired_len(&self) -> usize {
        self.desired.len()
    }

    pub fn apply(&mut self, cmd: RaftCommand) -> RaftResponse {
        self.expire_idempotency_ttl();
        let response = match cmd {
            RaftCommand::Idempotent {
                operation_id,
                request_digest,
                inner,
            } => self.apply_idempotent(operation_id, request_digest, *inner),
            other => self.apply_inner(other),
        };
        // Time-advancing commands expire receipts against the *new* committed clock.
        self.expire_idempotency_ttl();
        response
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
            while self.idempotency.len() >= self.idempotency_max_entries {
                self.evict_one_idempotency_by_key();
            }
            let recorded_at_ms = self.records.now_ms();
            self.idempotency.insert(
                operation_id,
                IdempotencyEntry {
                    request_digest,
                    response: response.clone(),
                    recorded_at_ms,
                },
            );
            self.note_idempotency_time(recorded_at_ms);
        }
        response
    }

    /// Drop expired receipts using committed time (fast-path when nothing can expire).
    fn expire_idempotency_ttl(&mut self) {
        let now = self.records.now_ms();
        let ttl = self.idempotency_ttl_ms;
        if self.idempotency_oldest_ms.is_none() && !self.idempotency.is_empty() {
            self.recompute_idempotency_oldest();
        }
        let Some(oldest) = self.idempotency_oldest_ms else {
            return;
        };
        if now.saturating_sub(oldest) <= ttl {
            return;
        }
        self.idempotency
            .retain(|_, e| now.saturating_sub(e.recorded_at_ms) <= ttl);
        self.recompute_idempotency_oldest();
    }

    /// Deterministic ceiling eviction: oldest `(recorded_at_ms, operation_id)` (STL-23).
    fn evict_one_idempotency_by_key(&mut self) {
        let Some(oldest_key) = self
            .idempotency
            .iter()
            .min_by_key(|(id, e)| (e.recorded_at_ms, *id))
            .map(|(id, _)| *id)
        else {
            return;
        };
        if let Some(removed) = self.idempotency.remove(&oldest_key) {
            if self.idempotency_oldest_ms == Some(removed.recorded_at_ms) {
                self.recompute_idempotency_oldest();
            }
        }
    }

    fn note_idempotency_time(&mut self, recorded_at_ms: u64) {
        self.idempotency_oldest_ms = Some(
            self.idempotency_oldest_ms
                .map(|o| o.min(recorded_at_ms))
                .unwrap_or(recorded_at_ms),
        );
    }

    fn recompute_idempotency_oldest(&mut self) {
        self.idempotency_oldest_ms = self.idempotency.values().map(|e| e.recorded_at_ms).min();
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
                if let Err(msg) = validate_desired_identity(&namespace, &app) {
                    return RaftResponse::Rejected(msg);
                }
                if payload.len() > DESIRED_MAX_PAYLOAD_BYTES {
                    return RaftResponse::Rejected(format!(
                        "desired payload {} exceeds max {}",
                        payload.len(),
                        DESIRED_MAX_PAYLOAD_BYTES
                    ));
                }
                // PROTOCOL §7 WorkloadsDesired ceiling stays aligned with the constant above.
                debug_assert_eq!(
                    DESIRED_MAX_PAYLOAD_BYTES,
                    KeyPrefix::WorkloadsDesired.max_payload()
                );

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

                let incoming = desired_entry_bytes(&key.0, &key.1, &payload);
                let previous = self
                    .desired
                    .get(&key)
                    .map(|e| desired_entry_bytes(&key.0, &key.1, &e.payload))
                    .unwrap_or(0);
                let is_new_key = !self.desired.contains_key(&key);
                if is_new_key && self.desired.len() >= self.desired_max_entries {
                    return RaftResponse::Rejected(format!(
                        "desired map full: {} entries (max {})",
                        self.desired.len(),
                        self.desired_max_entries
                    ));
                }
                let total_after = self
                    .desired_total_bytes()
                    .saturating_sub(previous)
                    .saturating_add(incoming);
                if total_after > self.desired_max_total_bytes {
                    return RaftResponse::Rejected(format!(
                        "desired byte budget exceeded: {total_after} > {}",
                        self.desired_max_total_bytes
                    ));
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

fn validate_desired_identity(namespace: &str, app: &str) -> Result<(), String> {
    Label::parse(namespace).map_err(|e| format!("invalid desired namespace: {e}"))?;
    Label::parse(app).map_err(|e| format!("invalid desired app: {e}"))?;
    Ok(())
}

fn desired_entry_bytes(namespace: &str, app: &str, payload: &[u8]) -> usize {
    namespace
        .len()
        .saturating_add(app.len())
        .saturating_add(payload.len())
}

impl ClusterState {
    fn desired_total_bytes(&self) -> usize {
        self.desired
            .iter()
            .map(|((ns, app), e)| desired_entry_bytes(ns, app, &e.payload))
            .fold(0usize, usize::saturating_add)
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

#[cfg(test)]
mod stl23_restore_tests {
    use super::*;
    use crate::records::Command;

    #[test]
    fn missing_oldest_index_still_expires_on_advance_time() {
        // Simulates serde(skip) loss of idempotency_oldest_ms after snapshot install.
        let mut state = ClusterState::new();
        let op = [1u8; 16];
        let resp = state.apply(RaftCommand::Idempotent {
            operation_id: op,
            request_digest: [2u8; 32],
            inner: Box::new(RaftCommand::Record(Command::AdvanceTime { now_ms: 1_000 })),
        });
        assert!(matches!(resp, RaftResponse::Applied(_)));
        assert_eq!(state.idempotency_len(), 1);

        state.idempotency_oldest_ms = None;

        let later = 1_000 + IDEMPOTENCY_TTL_MS + 1;
        let tick = state.apply(RaftCommand::Record(Command::AdvanceTime { now_ms: later }));
        assert!(matches!(tick, RaftResponse::Applied(_)));
        assert_eq!(
            state.idempotency_len(),
            0,
            "TTL must inspect receipts after oldest index is lost on restore"
        );
    }
}
