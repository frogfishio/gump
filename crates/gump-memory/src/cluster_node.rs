//! Live one-voter OpenRaft node over RAM stores (GUMP-N005 / C03–C07).
//!
//! Peers are not contacted — [`LoneNetworkFactory`] fails closed. Cluster intent
//! lives only in process RAM (D006); restart always begins empty.

use std::collections::BTreeSet;
use std::fmt;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use openraft::error::{InstallSnapshotError, RPCError, RaftError, Unreachable};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use openraft::{Config, Raft, ServerState};

use crate::cluster_state::{RaftCommand, RaftResponse};
use crate::ram_store::{MemoryNodeId, RamStateMachine, TypeConfig, ram_v2_stores};

/// Snapshot of live Raft + applied controller authority for status surfaces.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClusterStatusSnapshot {
    pub node_id: MemoryNodeId,
    pub current_leader: Option<MemoryNodeId>,
    pub voter_count: u32,
    pub controller_epoch: u64,
    pub controller_holder: Option<u64>,
    /// Always false — Gump creates no durable cluster state files (D006).
    pub durable_cluster_state: bool,
}

/// Process-local OpenRaft node with a dedicated Tokio runtime.
pub struct MemoryCluster {
    runtime: tokio::runtime::Runtime,
    raft: Raft<TypeConfig>,
    sm: RamStateMachine,
    node_id: MemoryNodeId,
}

impl fmt::Debug for MemoryCluster {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryCluster")
            .field("node_id", &self.node_id)
            .finish_non_exhaustive()
    }
}

impl MemoryCluster {
    /// Form a fresh one-voter cluster and acquire controller via Raft (not direct SM).
    pub fn bootstrap_one_voter(
        node_id: MemoryNodeId,
        controller_holder: u64,
    ) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .thread_name("gump-raft")
            .build()
            .map_err(|e| format!("raft runtime: {e}"))?;

        let (log, sm) = ram_v2_stores();
        let sm_handle = sm.clone();

        let raft = runtime.block_on(async {
            let config = Config {
                cluster_name: "gump".into(),
                heartbeat_interval: 50,
                election_timeout_min: 150,
                election_timeout_max: 300,
                ..Default::default()
            }
            .validate()
            .map_err(|e| format!("raft config: {e}"))?;
            let config = Arc::new(config);

            let raft = Raft::new(node_id, config, LoneNetworkFactory, log, sm)
                .await
                .map_err(|e| format!("raft new: {e}"))?;

            let mut members = BTreeSet::new();
            members.insert(node_id);
            raft.initialize(members)
                .await
                .map_err(|e| format!("raft initialize: {e}"))?;

            raft.wait(Some(Duration::from_secs(5)))
                .metrics(
                    |m| m.current_leader == Some(node_id) && m.state == ServerState::Leader,
                    "one-voter leader",
                )
                .await
                .map_err(|e| format!("raft wait leader: {e}"))?;

            let resp = raft
                .client_write(RaftCommand::AcquireController {
                    holder: controller_holder,
                })
                .await
                .map_err(|e| format!("raft acquire controller: {e}"))?;
            match resp.data {
                RaftResponse::Applied(o) if o.controller.is_some() => {}
                other => {
                    return Err(format!("expected Applied controller fence, got {other:?}"));
                }
            }

            Ok::<_, String>(raft)
        })?;

        Ok(Self {
            runtime,
            raft,
            sm: sm_handle,
            node_id,
        })
    }

    pub fn node_id(&self) -> MemoryNodeId {
        self.node_id
    }

    /// Mutate through the live Raft node (never call [`crate::ClusterState::apply`] here).
    pub fn client_write(&self, cmd: RaftCommand) -> Result<RaftResponse, String> {
        self.runtime.block_on(async {
            let resp = self
                .raft
                .client_write(cmd)
                .await
                .map_err(|e| format!("client_write: {e}"))?;
            Ok(resp.data)
        })
    }

    /// Linearizable-ish local read of applied SM after ensuring leadership.
    pub fn status_snapshot(&self) -> Result<ClusterStatusSnapshot, String> {
        self.runtime.block_on(async {
            self.raft
                .ensure_linearizable()
                .await
                .map_err(|e| format!("ensure_linearizable: {e}"))?;
            let metrics = self.raft.metrics().borrow().clone();
            let voter_count = metrics.membership_config.membership().voter_ids().count() as u32;
            let cluster = self.sm.cluster_state().await;
            let controller = cluster.controller();
            Ok(ClusterStatusSnapshot {
                node_id: self.node_id,
                current_leader: metrics.current_leader,
                voter_count,
                controller_epoch: controller.epoch(),
                controller_holder: controller.holder(),
                durable_cluster_state: false,
            })
        })
    }

    pub fn shutdown(&self) -> Result<(), String> {
        self.runtime.block_on(async {
            self.raft
                .shutdown()
                .await
                .map_err(|e| format!("raft shutdown: {e}"))
        })
    }
}

/// Network factory for a lone voter — any peer RPC is unreachable.
#[derive(Clone, Debug, Default)]
struct LoneNetworkFactory;

#[derive(Clone, Debug, Default)]
struct LoneNetwork;

impl RaftNetworkFactory<TypeConfig> for LoneNetworkFactory {
    type Network = LoneNetwork;

    async fn new_client(&mut self, _target: MemoryNodeId, _node: &()) -> Self::Network {
        LoneNetwork
    }
}

fn lone_unreachable<E>() -> RPCError<MemoryNodeId, (), E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    RPCError::Unreachable(Unreachable::new(&io::Error::new(
        io::ErrorKind::NotConnected,
        "lone voter has no peers",
    )))
}

impl RaftNetwork<TypeConfig> for LoneNetwork {
    async fn append_entries(
        &mut self,
        _rpc: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<
        AppendEntriesResponse<MemoryNodeId>,
        RPCError<MemoryNodeId, (), RaftError<MemoryNodeId>>,
    > {
        Err(lone_unreachable())
    }

    async fn install_snapshot(
        &mut self,
        _rpc: InstallSnapshotRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<
        InstallSnapshotResponse<MemoryNodeId>,
        RPCError<MemoryNodeId, (), RaftError<MemoryNodeId, InstallSnapshotError>>,
    > {
        Err(lone_unreachable())
    }

    async fn vote(
        &mut self,
        _rpc: VoteRequest<MemoryNodeId>,
        _option: RPCOption,
    ) -> Result<VoteResponse<MemoryNodeId>, RPCError<MemoryNodeId, (), RaftError<MemoryNodeId>>>
    {
        Err(lone_unreachable())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster_state::RaftCommand;
    use crate::records::Command;
    use std::fs;

    #[test]
    fn one_voter_bootstrap_and_raft_mutation() {
        let cluster = MemoryCluster::bootstrap_one_voter(1, 7).expect("bootstrap");
        let status = cluster.status_snapshot().expect("status");
        assert_eq!(status.voter_count, 1);
        assert_eq!(status.current_leader, Some(1));
        assert_eq!(status.controller_holder, Some(7));
        assert!(status.controller_epoch >= 1);
        assert!(!status.durable_cluster_state);

        let resp = cluster
            .client_write(RaftCommand::Record(Command::AdvanceTime { now_ms: 42 }))
            .expect("write");
        assert!(matches!(resp, RaftResponse::Applied(_)));

        cluster.shutdown().expect("shutdown");
    }

    #[test]
    fn bootstrap_leaves_no_files_in_sandbox() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sandbox = std::env::temp_dir().join(format!("gump-n005-nowrite-{nanos}"));
        fs::create_dir_all(&sandbox).unwrap();
        assert!(fs::read_dir(&sandbox).unwrap().next().is_none());

        let cluster = MemoryCluster::bootstrap_one_voter(1, 1).unwrap();
        let _ = cluster.status_snapshot().unwrap();
        cluster.shutdown().unwrap();

        assert!(
            fs::read_dir(&sandbox).unwrap().next().is_none(),
            "cluster must not create durable files under {:?}",
            sandbox
        );
        let _ = fs::remove_dir_all(sandbox);
    }
}
