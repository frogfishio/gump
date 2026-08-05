//! Authenticated telemetry batch relay to keepers (T04 / D011 / TELEMETRY.md §7).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::keeper::{select_keepers, NodeId, TARGET_KEEPER_REPLICAS};

/// Maximum records per authenticated batch.
pub const MAX_BATCH_RECORDS: usize = 256;

/// Default per-keeper byte budget for one shard window.
pub const DEFAULT_KEEPER_SHARD_BYTES: usize = 1024 * 1024;

/// Deduplication identity: execution + attempt + topic + sequence (TELEMETRY.md §12).
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct DedupId {
    pub execution_id: u64,
    pub attempt_id: u64,
    pub topic: String,
    pub sequence: u64,
}

/// Session proof presented with a batch (transport-authenticated; not a Raft fence).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchAuth {
    pub session_id: u64,
    /// Digest of attempt/fence material revalidated by the keeper (PROTOCOL §16).
    pub attempt_fence_digest: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayRecord {
    pub dedup: DedupId,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TelemetryBatch {
    pub shard_key: Vec<u8>,
    pub auth: BatchAuth,
    pub records: Vec<RelayRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelayError {
    EmptyBatch,
    BatchTooLarge { len: usize, max: usize },
    Unauthorized { session_id: u64 },
    NotAKeeper { node: NodeId },
}

impl std::fmt::Display for RelayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyBatch => write!(f, "empty telemetry batch"),
            Self::BatchTooLarge { len, max } => {
                write!(f, "batch length {len} exceeds max {max}")
            }
            Self::Unauthorized { session_id } => {
                write!(f, "unauthorized session {session_id}")
            }
            Self::NotAKeeper { node } => write!(f, "node {node} is not a selected keeper"),
        }
    }
}

impl std::error::Error for RelayError {}

#[derive(Clone, Debug)]
struct StoredRecord {
    record: RelayRecord,
    bytes: usize,
}

/// Bounded in-memory keeper window for one node.
#[derive(Clone, Debug)]
pub struct KeeperStore {
    pub node_id: NodeId,
    max_bytes: usize,
    entries: VecDeque<StoredRecord>,
    total_bytes: usize,
    seen: BTreeSet<DedupId>,
    accepted: u64,
    dropped_oldest: u64,
    rejected_auth: u64,
}

impl KeeperStore {
    pub fn new(node_id: NodeId, max_bytes: usize) -> Self {
        Self {
            node_id,
            max_bytes: max_bytes.max(1),
            entries: VecDeque::new(),
            total_bytes: 0,
            seen: BTreeSet::new(),
            accepted: 0,
            dropped_oldest: 0,
            rejected_auth: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    pub fn accepted(&self) -> u64 {
        self.accepted
    }

    pub fn dropped_oldest(&self) -> u64 {
        self.dropped_oldest
    }

    pub fn contains_dedup(&self, id: &DedupId) -> bool {
        self.seen.contains(id)
    }

    pub fn accept(
        &mut self,
        batch: &TelemetryBatch,
        expected_auth: &BatchAuth,
    ) -> Result<usize, RelayError> {
        if batch.records.is_empty() {
            return Err(RelayError::EmptyBatch);
        }
        if batch.records.len() > MAX_BATCH_RECORDS {
            return Err(RelayError::BatchTooLarge {
                len: batch.records.len(),
                max: MAX_BATCH_RECORDS,
            });
        }
        if &batch.auth != expected_auth {
            self.rejected_auth += 1;
            return Err(RelayError::Unauthorized {
                session_id: batch.auth.session_id,
            });
        }

        let mut newly = 0usize;
        for rec in &batch.records {
            if self.seen.contains(&rec.dedup) {
                continue; // dedupe — not a user-visible duplicate
            }
            let bytes = rec.payload.len().saturating_add(32);
            self.evict_until(bytes);
            self.seen.insert(rec.dedup.clone());
            self.entries.push_back(StoredRecord {
                record: rec.clone(),
                bytes,
            });
            self.total_bytes = self.total_bytes.saturating_add(bytes);
            self.accepted += 1;
            newly += 1;
        }
        Ok(newly)
    }

    /// Transfer retained records into another store (keeper membership change).
    pub fn transfer_into(&self, dst: &mut KeeperStore, expected_auth: &BatchAuth) -> usize {
        let mut n = 0;
        for e in &self.entries {
            let batch = TelemetryBatch {
                shard_key: Vec::new(),
                auth: expected_auth.clone(),
                records: vec![e.record.clone()],
            };
            if dst.accept(&batch, expected_auth).unwrap_or(0) > 0 {
                n += 1;
            }
        }
        n
    }

    fn evict_until(&mut self, needed: usize) {
        while self.total_bytes.saturating_add(needed) > self.max_bytes && !self.entries.is_empty()
        {
            if let Some(old) = self.entries.pop_front() {
                self.total_bytes = self.total_bytes.saturating_sub(old.bytes);
                self.seen.remove(&old.record.dedup);
                self.dropped_oldest += 1;
            }
        }
    }
}

/// In-memory multi-keeper relay mesh for simulation (node-loss / transfer / overflow).
#[derive(Clone, Debug)]
pub struct RelayMesh {
    nodes: Vec<NodeId>,
    stores: BTreeMap<NodeId, KeeperStore>,
    auth: BatchAuth,
    shard_bytes: usize,
}

impl RelayMesh {
    pub fn new(nodes: Vec<NodeId>, auth: BatchAuth, shard_bytes: usize) -> Self {
        let mut stores = BTreeMap::new();
        for n in &nodes {
            stores.insert(*n, KeeperStore::new(*n, shard_bytes));
        }
        Self {
            nodes,
            stores,
            auth,
            shard_bytes,
        }
    }

    pub fn auth(&self) -> &BatchAuth {
        &self.auth
    }

    pub fn nodes(&self) -> &[NodeId] {
        &self.nodes
    }

    pub fn store(&self, node: NodeId) -> Option<&KeeperStore> {
        self.stores.get(&node)
    }

    pub fn keepers_for(&self, shard_key: &[u8]) -> Vec<NodeId> {
        select_keepers(shard_key, &self.nodes)
    }

    /// Forward a batch to all currently selected keepers for its shard.
    pub fn relay(&mut self, batch: &TelemetryBatch) -> Result<usize, RelayError> {
        let keepers = select_keepers(&batch.shard_key, &self.nodes);
        if keepers.is_empty() {
            return Err(RelayError::NotAKeeper { node: 0 });
        }
        let mut total = 0;
        for k in keepers {
            let store = self
                .stores
                .get_mut(&k)
                .ok_or(RelayError::NotAKeeper { node: k })?;
            total += store.accept(batch, &self.auth)?;
        }
        Ok(total)
    }

    /// Remove a failed node; retained records on survivors stay available.
    pub fn lose_node(&mut self, node: NodeId) {
        self.nodes.retain(|n| *n != node);
        self.stores.remove(&node);
    }

    /// Add a replacement node and transfer shard windows from current keepers.
    pub fn transfer_join(&mut self, node: NodeId, shard_key: &[u8]) -> usize {
        if !self.nodes.contains(&node) {
            self.nodes.push(node);
            self.nodes.sort_unstable();
            self.stores
                .insert(node, KeeperStore::new(node, self.shard_bytes));
        }
        let keepers = select_keepers(shard_key, &self.nodes);
        if !keepers.contains(&node) {
            return 0;
        }
        // Copy from any existing store that still holds the shard.
        let donors: Vec<NodeId> = self.stores.keys().copied().filter(|n| *n != node).collect();
        let mut moved = 0;
        let auth = self.auth.clone();
        for d in donors {
            let snapshot = self.stores.get(&d).cloned();
            if let Some(src) = snapshot {
                if let Some(dst) = self.stores.get_mut(&node) {
                    moved += src.transfer_into(dst, &auth);
                }
            }
        }
        let _ = TARGET_KEEPER_REPLICAS;
        moved
    }
}
