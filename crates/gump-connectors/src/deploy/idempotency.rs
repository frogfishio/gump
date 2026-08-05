//! Bounded idempotency cache for deploy mutations (PROTOCOL.md §15 / D014).

use std::collections::BTreeMap;

/// Cached mutation response keyed by operation ID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdempotencyRecord {
    pub principal: String,
    pub request_digest: [u8; 32],
    pub result_digest: [u8; 32],
    pub cluster_revision: u64,
    pub recorded_at_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IdempotencyError {
    Conflict {
        operation_id: [u8; 16],
    },
}

impl std::fmt::Display for IdempotencyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict { .. } => write!(f, "idempotency conflict: different request digest"),
        }
    }
}

impl std::error::Error for IdempotencyError {}

/// Retains outcomes for 24h or 100_000 ops, whichever binds first (D014).
#[derive(Debug)]
pub struct IdempotencyCache {
    by_op: BTreeMap<[u8; 16], IdempotencyRecord>,
    max_entries: usize,
    ttl_ms: u64,
}

impl Default for IdempotencyCache {
    fn default() -> Self {
        Self::new()
    }
}

impl IdempotencyCache {
    pub const DEFAULT_MAX_ENTRIES: usize = 100_000;
    pub const DEFAULT_TTL_MS: u64 = 24 * 60 * 60 * 1_000;

    pub fn new() -> Self {
        Self {
            by_op: BTreeMap::new(),
            max_entries: Self::DEFAULT_MAX_ENTRIES,
            ttl_ms: Self::DEFAULT_TTL_MS,
        }
    }

    pub fn with_limits(max_entries: usize, ttl_ms: u64) -> Self {
        Self {
            by_op: BTreeMap::new(),
            max_entries: max_entries.max(1),
            ttl_ms,
        }
    }

    pub fn len(&self) -> usize {
        self.by_op.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_op.is_empty()
    }

    pub fn get(
        &mut self,
        operation_id: &[u8; 16],
        now_ms: u64,
    ) -> Option<&IdempotencyRecord> {
        self.expire(now_ms);
        self.by_op.get(operation_id)
    }

    /// Lookup that enforces same request digest (else CONFLICT).
    pub fn check(
        &mut self,
        operation_id: &[u8; 16],
        request_digest: &[u8; 32],
        now_ms: u64,
    ) -> Result<Option<IdempotencyRecord>, IdempotencyError> {
        self.expire(now_ms);
        match self.by_op.get(operation_id) {
            None => Ok(None),
            Some(rec) if &rec.request_digest == request_digest => Ok(Some(rec.clone())),
            Some(_) => Err(IdempotencyError::Conflict {
                operation_id: *operation_id,
            }),
        }
    }

    pub fn put(&mut self, operation_id: [u8; 16], record: IdempotencyRecord, now_ms: u64) {
        self.expire(now_ms);
        while self.by_op.len() >= self.max_entries {
            if let Some(oldest) = self.by_op.keys().next().copied() {
                self.by_op.remove(&oldest);
            } else {
                break;
            }
        }
        self.by_op.insert(operation_id, record);
    }

    fn expire(&mut self, now_ms: u64) {
        let ttl = self.ttl_ms;
        self.by_op
            .retain(|_, rec| now_ms.saturating_sub(rec.recorded_at_ms) <= ttl);
    }
}
