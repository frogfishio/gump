//! Watch history and compaction (PROTOCOL.md §8).

use crate::records::key::RecordKey;

/// Retain at most 10,000 revisions of watch history.
pub const MAX_WATCH_REVISIONS: usize = 10_000;

/// Retain at most 10 minutes of watch history (milliseconds).
pub const MAX_WATCH_AGE_MS: u64 = 10 * 60 * 1000;

/// Ordered change visible to watchers after a revision.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum WatchChange {
    Put {
        key: RecordKey,
        revision: u64,
        digest: [u8; 32],
    },
    Delete {
        key: RecordKey,
        revision: u64,
    },
    /// Lease revoked (expiry or explicit revoke).
    LeaseRevoked {
        lease_id: u64,
        revision: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WatchBatch {
    pub revision: u64,
    pub at_ms: u64,
    pub changes: Vec<WatchChange>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Compacted {
    /// Highest revision no longer available; resume watches with `after = compaction_floor`.
    pub compaction_floor: u64,
}

impl std::fmt::Display for Compacted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "COMPACTED: resume after linearizable relist from revision {}",
            self.compaction_floor
        )
    }
}

impl std::error::Error for Compacted {}

/// Bounded committed-change log for watches.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct WatchHistory {
    /// Highest revision that has been compacted away (unavailable).
    compacted_through: u64,
    batches: Vec<WatchBatch>,
    /// Approximate bytes charged toward the history budget cap.
    approx_bytes: u64,
}

impl WatchHistory {
    pub fn floor(&self) -> u64 {
        self.compacted_through
    }

    pub fn approx_bytes(&self) -> u64 {
        self.approx_bytes
    }

    pub fn len(&self) -> usize {
        self.batches.len()
    }

    pub fn is_empty(&self) -> bool {
        self.batches.is_empty()
    }

    /// Compact away revisions `<= through`.
    pub fn compact_through(&mut self, through: u64) {
        self.compacted_through = self.compacted_through.max(through);
        self.prune_compacted();
    }

    pub fn push(&mut self, batch: WatchBatch, now_ms: u64, history_budget: u64) {
        self.approx_bytes = self.approx_bytes.saturating_add(batch_bytes(&batch));
        self.batches.push(batch);
        self.retain(now_ms, history_budget);
    }

    /// Prune by age (from `now_ms`), revision count, compacted floor, and history budget.
    pub fn retain(&mut self, now_ms: u64, history_budget: u64) {
        self.prune_compacted();
        let cutoff = now_ms.saturating_sub(MAX_WATCH_AGE_MS);
        while let Some(first) = self.batches.first() {
            if first.at_ms >= cutoff {
                break;
            }
            self.compacted_through = self.compacted_through.max(first.revision);
            self.batches.remove(0);
        }
        while self.batches.len() > MAX_WATCH_REVISIONS {
            if let Some(dropped) = self.batches.first() {
                self.compacted_through = self.compacted_through.max(dropped.revision);
            }
            self.batches.remove(0);
        }
        while self.recompute_bytes() > history_budget && !self.batches.is_empty() {
            if let Some(dropped) = self.batches.first() {
                self.compacted_through = self.compacted_through.max(dropped.revision);
            }
            self.batches.remove(0);
        }
        self.recompute_bytes();
    }

    /// Events with revision strictly greater than `after`.
    pub fn watch_after(&self, after: u64) -> Result<Vec<WatchBatch>, Compacted> {
        if after < self.compacted_through {
            return Err(Compacted {
                compaction_floor: self.compacted_through,
            });
        }
        Ok(self
            .batches
            .iter()
            .filter(|b| b.revision > after)
            .cloned()
            .collect())
    }

    fn prune_compacted(&mut self) {
        let through = self.compacted_through;
        self.batches.retain(|b| b.revision > through);
        self.recompute_bytes();
    }

    fn recompute_bytes(&mut self) -> u64 {
        self.approx_bytes = self.batches.iter().map(batch_bytes).sum();
        self.approx_bytes
    }
}

fn batch_bytes(batch: &WatchBatch) -> u64 {
    let mut n = 16u64; // revision + at_ms
    for c in &batch.changes {
        n += match c {
            WatchChange::Put { key, .. } => 40 + key.to_string().len() as u64,
            WatchChange::Delete { key, .. } => 16 + key.to_string().len() as u64,
            WatchChange::LeaseRevoked { .. } => 24,
        };
    }
    n
}
