//! Gump-owned RAM OpenRaft v2 stores (C03 / D001). No `std::fs` — buffers only.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::sync::Arc;

use openraft::storage::{LogFlushed, RaftLogStorage, RaftStateMachine, Snapshot};
use openraft::{
    Entry, EntryPayload, LogId, LogState, OptionalSend, RaftLogId, RaftLogReader,
    RaftSnapshotBuilder, RaftTypeConfig, SnapshotMeta, StorageError, StorageIOError,
    StoredMembership, Vote,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// OpenRaft node id (`u64`, collision-checked at formation — PROTOCOL.md §6).
pub type MemoryNodeId = u64;

/// Placeholder app request until C04 typed records land.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClientRequest {
    pub client: String,
    pub serial: u64,
    pub status: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClientResponse(pub Option<String>);

openraft::declare_raft_types!(
    /// Type configuration for the RAM adapter (C03).
    pub TypeConfig:
        D = ClientRequest,
        R = ClientResponse,
        Node = (),
);

#[derive(Debug)]
struct RamSnapshot {
    meta: SnapshotMeta<MemoryNodeId, ()>,
    data: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
struct StateMachineData {
    last_applied_log: Option<LogId<MemoryNodeId>>,
    last_membership: StoredMembership<MemoryNodeId, ()>,
    client_serial_responses: HashMap<String, (u64, Option<String>)>,
    client_status: HashMap<String, String>,
}

#[derive(Default)]
struct LogInner {
    last_purged_log_id: Option<LogId<MemoryNodeId>>,
    committed: Option<LogId<MemoryNodeId>>,
    log: BTreeMap<u64, Entry<TypeConfig>>,
    vote: Option<Vote<MemoryNodeId>>,
}

#[derive(Default)]
struct SmInner {
    sm: StateMachineData,
    snapshot_idx: u64,
    current_snapshot: Option<RamSnapshot>,
}

/// RAM log + vote store (OpenRaft v2 [`RaftLogStorage`]).
#[derive(Clone, Default)]
pub struct RamLogStore {
    inner: Arc<RwLock<LogInner>>,
}

/// RAM state machine + snapshot store (OpenRaft v2 [`RaftStateMachine`]).
#[derive(Clone, Default)]
pub struct RamStateMachine {
    inner: Arc<RwLock<SmInner>>,
}

/// Combined factory used by the OpenRaft storage test suite.
pub struct RamStore {
    log: RamLogStore,
    sm: RamStateMachine,
}

impl RamLogStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RamStateMachine {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RamStore {
    pub fn new() -> Self {
        Self {
            log: RamLogStore::new(),
            sm: RamStateMachine::new(),
        }
    }

    pub async fn new_async() -> Self {
        Self::new()
    }

    pub fn split(self) -> (RamLogStore, RamStateMachine) {
        (self.log, self.sm)
    }
}

impl Default for RamStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a fresh RAM log-store / state-machine pair (D001 v2 surface).
pub fn ram_v2_stores() -> (RamLogStore, RamStateMachine) {
    RamStore::new().split()
}

impl RaftLogReader<TypeConfig> for RamLogStore {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<TypeConfig>>, StorageError<MemoryNodeId>> {
        let inner = self.inner.read().await;
        Ok(inner.log.range(range).map(|(_, e)| e.clone()).collect())
    }
}

impl RaftLogStorage<TypeConfig> for RamLogStore {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, StorageError<MemoryNodeId>> {
        let inner = self.inner.read().await;
        let last = inner.log.iter().next_back().map(|(_, e)| *e.get_log_id());
        let last_purged = inner.last_purged_log_id;
        let last_log_id = match last {
            None => last_purged,
            Some(x) => Some(x),
        };
        Ok(LogState {
            last_purged_log_id: last_purged,
            last_log_id,
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn save_vote(&mut self, vote: &Vote<MemoryNodeId>) -> Result<(), StorageError<MemoryNodeId>> {
        self.inner.write().await.vote = Some(*vote);
        Ok(())
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<MemoryNodeId>>, StorageError<MemoryNodeId>> {
        Ok(self.inner.read().await.vote)
    }

    async fn save_committed(
        &mut self,
        committed: Option<LogId<MemoryNodeId>>,
    ) -> Result<(), StorageError<MemoryNodeId>> {
        self.inner.write().await.committed = committed;
        Ok(())
    }

    async fn read_committed(&mut self) -> Result<Option<LogId<MemoryNodeId>>, StorageError<MemoryNodeId>> {
        Ok(self.inner.read().await.committed)
    }

    async fn append<I>(
        &mut self,
        entries: I,
        callback: LogFlushed<TypeConfig>,
    ) -> Result<(), StorageError<MemoryNodeId>>
    where
        I: IntoIterator<Item = Entry<TypeConfig>> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        {
            let mut inner = self.inner.write().await;
            for entry in entries {
                inner.log.insert(entry.log_id.index, entry);
            }
        }
        // RAM "persistence" completes immediately.
        callback.log_io_completed(Ok(()));
        Ok(())
    }

    async fn truncate(&mut self, log_id: LogId<MemoryNodeId>) -> Result<(), StorageError<MemoryNodeId>> {
        let mut inner = self.inner.write().await;
        let keys: Vec<u64> = inner.log.range(log_id.index..).map(|(k, _)| *k).collect();
        for key in keys {
            inner.log.remove(&key);
        }
        Ok(())
    }

    async fn purge(&mut self, log_id: LogId<MemoryNodeId>) -> Result<(), StorageError<MemoryNodeId>> {
        let mut inner = self.inner.write().await;
        assert!(inner.last_purged_log_id <= Some(log_id));
        inner.last_purged_log_id = Some(log_id);
        let keys: Vec<u64> = inner.log.range(..=log_id.index).map(|(k, _)| *k).collect();
        for key in keys {
            inner.log.remove(&key);
        }
        Ok(())
    }
}

impl RaftSnapshotBuilder<TypeConfig> for RamStateMachine {
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>, StorageError<MemoryNodeId>> {
        let mut inner = self.inner.write().await;
        let data = serde_json::to_vec(&inner.sm)
            .map_err(|e| StorageIOError::read_state_machine(&e))?;
        let last_applied_log = inner.sm.last_applied_log;
        let last_membership = inner.sm.last_membership.clone();
        inner.snapshot_idx += 1;
        let snapshot_idx = inner.snapshot_idx;
        let snapshot_id = if let Some(last) = last_applied_log {
            format!("{}-{}-{}", last.leader_id, last.index, snapshot_idx)
        } else {
            format!("--{snapshot_idx}")
        };
        let meta = SnapshotMeta {
            last_log_id: last_applied_log,
            last_membership,
            snapshot_id,
        };
        inner.current_snapshot = Some(RamSnapshot {
            meta: meta.clone(),
            data: data.clone(),
        });
        Ok(Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(data)),
        })
    }
}

impl RaftStateMachine<TypeConfig> for RamStateMachine {
    type SnapshotBuilder = Self;

    async fn applied_state(
        &mut self,
    ) -> Result<(Option<LogId<MemoryNodeId>>, StoredMembership<MemoryNodeId, ()>), StorageError<MemoryNodeId>>
    {
        let inner = self.inner.read().await;
        Ok((inner.sm.last_applied_log, inner.sm.last_membership.clone()))
    }

    async fn apply<I>(&mut self, entries: I) -> Result<Vec<ClientResponse>, StorageError<MemoryNodeId>>
    where
        I: IntoIterator<Item = Entry<TypeConfig>> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let mut res = Vec::new();
        let mut inner = self.inner.write().await;
        for entry in entries {
            inner.sm.last_applied_log = Some(entry.log_id);
            match &entry.payload {
                EntryPayload::Blank => res.push(ClientResponse(None)),
                EntryPayload::Normal(data) => {
                    if let Some((serial, r)) = inner.sm.client_serial_responses.get(&data.client) {
                        if serial == &data.serial {
                            res.push(ClientResponse(r.clone()));
                            continue;
                        }
                    }
                    let previous = inner
                        .sm
                        .client_status
                        .insert(data.client.clone(), data.status.clone());
                    inner
                        .sm
                        .client_serial_responses
                        .insert(data.client.clone(), (data.serial, previous.clone()));
                    res.push(ClientResponse(previous));
                }
                EntryPayload::Membership(mem) => {
                    inner.sm.last_membership =
                        StoredMembership::new(Some(entry.log_id), mem.clone());
                    res.push(ClientResponse(None));
                }
            }
        }
        Ok(res)
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }

    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<Box<<TypeConfig as RaftTypeConfig>::SnapshotData>, StorageError<MemoryNodeId>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<MemoryNodeId, ()>,
        snapshot: Box<<TypeConfig as RaftTypeConfig>::SnapshotData>,
    ) -> Result<(), StorageError<MemoryNodeId>> {
        let new_snapshot = RamSnapshot {
            meta: meta.clone(),
            data: snapshot.into_inner(),
        };
        let new_sm: StateMachineData = serde_json::from_slice(&new_snapshot.data)
            .map_err(|e| StorageIOError::read_snapshot(Some(new_snapshot.meta.signature()), &e))?;
        let mut inner = self.inner.write().await;
        inner.sm = new_sm;
        inner.current_snapshot = Some(new_snapshot);
        Ok(())
    }

    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<Snapshot<TypeConfig>>, StorageError<MemoryNodeId>> {
        match &self.inner.read().await.current_snapshot {
            Some(snapshot) => Ok(Some(Snapshot {
                meta: snapshot.meta.clone(),
                snapshot: Box::new(Cursor::new(snapshot.data.clone())),
            })),
            None => Ok(None),
        }
    }
}
