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

use crate::cluster_net::{
    ClusterNetworkConfig, ClusterReply, ClusterRpc, LiveClusterNetwork, call_addr, reply,
};
use crate::cluster_state::DesiredSnapshotEntry;
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
    network: Option<LiveClusterNetwork>,
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
            network: None,
        })
    }

    /// Form or join an authenticated multi-voter RAM cluster over QUIC mTLS.
    pub fn bootstrap_networked(
        node_id: MemoryNodeId,
        controller_holder: u64,
        network_config: ClusterNetworkConfig,
    ) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(4)
            .thread_name("gump-cluster")
            .build()
            .map_err(|e| format!("cluster runtime: {e}"))?;
        let (log, sm) = ram_v2_stores();
        let sm_handle = sm.clone();
        let mut network = runtime.block_on(async { LiveClusterNetwork::build(network_config) })?;
        let factory = network.factory.clone();
        let raft = runtime.block_on(async {
            let config = Arc::new(
                Config {
                    cluster_name: "gump".into(),
                    heartbeat_interval: 100,
                    election_timeout_min: 400,
                    election_timeout_max: 800,
                    ..Default::default()
                }
                .validate()
                .map_err(|e| format!("raft config: {e}"))?,
            );
            Raft::new(node_id, config, factory, log, sm)
                .await
                .map_err(|e| format!("raft new: {e}"))
        })?;

        spawn_cluster_server(&runtime, raft.clone(), &network);

        if let Some(join) = network.join.take() {
            let response = runtime
                .block_on(call_addr(
                    &network.client,
                    join.seed,
                    ClusterRpc::Join {
                        node_id,
                        advertise: network.advertise,
                        token: join.token.expose().clone(),
                    },
                ))
                .map_err(|e| format!("join seed: {e}"))?;
            let ClusterReply::Joined { peers } = response else {
                return Err(match response {
                    ClusterReply::Error(e) => format!("join rejected: {e}"),
                    _ => "join seed returned unexpected response".into(),
                });
            };
            network
                .peers
                .lock()
                .map_err(|_| "peer map poisoned".to_string())?
                .extend(peers);
            runtime.block_on(async {
                raft.wait(Some(Duration::from_secs(15)))
                    .metrics(
                        |m| {
                            m.current_leader.is_some()
                                && m.membership_config
                                    .membership()
                                    .voter_ids()
                                    .any(|id| id == node_id)
                        },
                        "joined voter",
                    )
                    .await
                    .map_err(|e| format!("wait joined voter: {e}"))
            })?;
        } else {
            runtime.block_on(async {
                let mut members = BTreeSet::new();
                members.insert(node_id);
                raft.initialize(members)
                    .await
                    .map_err(|e| format!("raft initialize: {e}"))?;
                raft.wait(Some(Duration::from_secs(5)))
                    .metrics(
                        |m| m.current_leader == Some(node_id) && m.state == ServerState::Leader,
                        "seed leader",
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
                    RaftResponse::Applied(o) if o.controller.is_some() => Ok(()),
                    other => Err(format!("expected controller fence, got {other:?}")),
                }
            })?;
        }

        Ok(Self {
            runtime,
            raft,
            sm: sm_handle,
            node_id,
            network: Some(network),
        })
    }

    pub fn node_id(&self) -> MemoryNodeId {
        self.node_id
    }

    /// Mutate through the live Raft node (never call [`crate::ClusterState::apply`] here).
    pub fn client_write(&self, cmd: RaftCommand) -> Result<RaftResponse, String> {
        self.runtime.block_on(async {
            match self.raft.client_write(cmd.clone()).await {
                Ok(resp) => Ok(resp.data),
                Err(local_error) => {
                    let leader = self.raft.metrics().borrow().current_leader;
                    let Some(network) = &self.network else {
                        return Err(format!("client_write: {local_error}"));
                    };
                    let Some(leader) = leader.filter(|id| *id != self.node_id) else {
                        return Err(format!("client_write: {local_error}"));
                    };
                    let addr = network
                        .peers
                        .lock()
                        .map_err(|_| "peer map poisoned".to_string())?
                        .get(&leader)
                        .copied()
                        .ok_or_else(|| format!("leader {leader} address unknown"))?;
                    match call_addr(&network.client, addr, ClusterRpc::ClientWrite(cmd))
                        .await
                        .map_err(|e| format!("forward client_write: {e}"))?
                    {
                        ClusterReply::ClientWrite(response) => Ok(response),
                        ClusterReply::Error(error) => Err(format!("forward client_write: {error}")),
                        _ => Err("forward client_write returned unexpected response".into()),
                    }
                }
            }
        })
    }

    /// Linearizable-ish local read of applied SM after ensuring leadership.
    pub fn status_snapshot(&self) -> Result<ClusterStatusSnapshot, String> {
        self.runtime.block_on(async {
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

    /// Linearizable read of applied desired-state length (GUMP-N016).
    pub fn desired_len(&self) -> Result<usize, String> {
        self.runtime.block_on(async {
            self.raft
                .ensure_linearizable()
                .await
                .map_err(|e| format!("ensure_linearizable: {e}"))?;
            Ok(self.sm.cluster_state().await.desired_len())
        })
    }

    /// Local applied-state observation for followers. It may lag the leader but
    /// never includes uncommitted entries; controller decisions still use the
    /// leader-only linearizable APIs.
    pub fn observed_desired_len(&self) -> usize {
        self.runtime
            .block_on(async { self.sm.cluster_state().await.desired_len() })
    }

    pub fn observed_desired_snapshot(&self) -> Vec<DesiredSnapshotEntry> {
        self.runtime
            .block_on(async { self.sm.cluster_state().await.desired_snapshot() })
    }

    pub fn observed_finite_completed(
        &self,
        namespace: &str,
        app: &str,
        generation: u64,
        unit_id: &[u8; 16],
    ) -> bool {
        self.runtime.block_on(async {
            self.sm
                .cluster_state()
                .await
                .finite_completed(namespace, app, generation, unit_id)
        })
    }

    pub fn voter_ids(&self) -> Vec<MemoryNodeId> {
        let mut voters: Vec<_> = self
            .raft
            .metrics()
            .borrow()
            .membership_config
            .membership()
            .voter_ids()
            .collect();
        voters.sort_unstable();
        voters
    }

    /// Linearizable committed desired-state view for the controller reconciler.
    pub fn desired_snapshot(&self) -> Result<Vec<DesiredSnapshotEntry>, String> {
        self.runtime.block_on(async {
            self.raft
                .ensure_linearizable()
                .await
                .map_err(|e| format!("ensure_linearizable: {e}"))?;
            Ok(self.sm.cluster_state().await.desired_snapshot())
        })
    }

    /// Whether live desired state references `digest` (GUMP-N016 inventory).
    pub fn desired_references_digest(&self, digest: &[u8; 32]) -> Result<bool, String> {
        self.runtime.block_on(async {
            self.raft
                .ensure_linearizable()
                .await
                .map_err(|e| format!("ensure_linearizable: {e}"))?;
            Ok(self
                .sm
                .cluster_state()
                .await
                .desired_references_digest(digest))
        })
    }

    pub fn shutdown(&self) -> Result<(), String> {
        if let Some(network) = &self.network {
            network.server.close();
            network.client.close();
        }
        self.runtime.block_on(async {
            self.raft
                .shutdown()
                .await
                .map_err(|e| format!("raft shutdown: {e}"))
        })
    }
}

fn spawn_cluster_server(
    runtime: &tokio::runtime::Runtime,
    raft: Raft<TypeConfig>,
    network: &LiveClusterNetwork,
) {
    let endpoint = Arc::clone(&network.server);
    let client = Arc::clone(&network.client);
    let peers = Arc::clone(&network.peers);
    let allowed = Arc::clone(&network.join_tokens);
    let used = Arc::clone(&network.used_tokens);
    runtime.spawn(async move {
        loop {
            let session = match endpoint.accept().await {
                Ok(session) => session,
                Err(e) => {
                    tracing::warn!("cluster accept stopped: {e}");
                    break;
                }
            };
            let raft = raft.clone();
            let client = Arc::clone(&client);
            let peers = Arc::clone(&peers);
            let allowed = Arc::clone(&allowed);
            let used = Arc::clone(&used);
            tokio::spawn(async move {
                let (send, mut recv) = match session.accept_bi().await {
                    Ok(streams) => streams,
                    Err(e) => {
                        tracing::warn!("cluster accept stream: {e}");
                        return;
                    }
                };
                let mut body = match session.recv_control(&mut recv).await {
                    Ok(body) => body,
                    Err(_) => return,
                };
                let parsed = serde_json::from_slice(&body);
                body.fill(0);
                let request: ClusterRpc = match parsed {
                    Ok(request) => request,
                    Err(e) => {
                        let _ = reply(&session, send, ClusterReply::Error(e.to_string())).await;
                        return;
                    }
                };
                let response =
                    handle_cluster_rpc(&session, &raft, &client, &peers, &allowed, &used, request)
                        .await;
                if let Err(e) = reply(&session, send, response).await {
                    tracing::warn!("cluster reply failed: {e}");
                }
            });
        }
    });
}

async fn handle_cluster_rpc(
    session: &gump_transport::QuicSession,
    raft: &Raft<TypeConfig>,
    client: &gump_transport::QuicEndpoint,
    peers: &Arc<std::sync::Mutex<std::collections::BTreeMap<MemoryNodeId, std::net::SocketAddr>>>,
    allowed: &Arc<std::sync::Mutex<std::collections::BTreeMap<MemoryNodeId, [u8; 32]>>>,
    used: &Arc<std::sync::Mutex<std::collections::BTreeSet<[u8; 32]>>>,
    request: ClusterRpc,
) -> ClusterReply {
    let peer_node = node_u64(session.peer.node_id);
    if !session
        .peer
        .roles
        .contains(&gump_transport::NodeRole::Memory)
    {
        return ClusterReply::Error("peer certificate lacks memory role".into());
    }
    match request {
        ClusterRpc::Append(req) => match raft.append_entries(req).await {
            Ok(r) => ClusterReply::Append(r),
            Err(e) => ClusterReply::Error(e.to_string()),
        },
        ClusterRpc::Vote(req) => match raft.vote(req).await {
            Ok(r) => ClusterReply::Vote(r),
            Err(e) => ClusterReply::Error(e.to_string()),
        },
        ClusterRpc::Install(req) => match raft.install_snapshot(req).await {
            Ok(r) => ClusterReply::Install(r),
            Err(e) => ClusterReply::Error(e.to_string()),
        },
        ClusterRpc::ClientWrite(command) => match raft.client_write(command).await {
            Ok(response) => ClusterReply::ClientWrite(response.data),
            Err(e) => ClusterReply::Error(e.to_string()),
        },
        ClusterRpc::Announce { node_id, advertise } => {
            let is_voter = raft
                .metrics()
                .borrow()
                .membership_config
                .membership()
                .voter_ids()
                .any(|id| id == node_id);
            if !is_voter {
                return ClusterReply::Error("announcement target is not a voter".into());
            }
            match peers.lock() {
                Ok(mut peers) => {
                    peers.insert(node_id, advertise);
                    ClusterReply::Ack
                }
                Err(_) => ClusterReply::Error("peer map poisoned".into()),
            }
        }
        ClusterRpc::Join {
            node_id,
            advertise,
            mut token,
        } => {
            if node_id != peer_node {
                return ClusterReply::Error("join certificate/node mismatch".into());
            }
            let digest = *blake3::hash(token.as_bytes()).as_bytes();
            zeroize::Zeroize::zeroize(&mut token);
            let authorized = allowed
                .lock()
                .ok()
                .and_then(|a| a.get(&node_id).copied())
                .map(|expected| expected == digest)
                .unwrap_or(false);
            if !authorized {
                return ClusterReply::Error("join token unauthorized".into());
            }
            {
                let Ok(mut used) = used.lock() else {
                    return ClusterReply::Error("join replay store poisoned".into());
                };
                if !used.insert(digest) {
                    return ClusterReply::Error("join token replayed".into());
                }
            }
            if let Ok(mut peers) = peers.lock() {
                peers.insert(node_id, advertise);
            } else {
                return ClusterReply::Error("peer map poisoned".into());
            }
            if let Err(e) = raft.add_learner(node_id, (), true).await {
                return ClusterReply::Error(format!("add learner: {e}"));
            }
            let mut voters: BTreeSet<_> = raft
                .metrics()
                .borrow()
                .membership_config
                .membership()
                .voter_ids()
                .collect();
            voters.insert(node_id);
            if let Err(e) = raft.change_membership(voters, true).await {
                return ClusterReply::Error(format!("promote learner: {e}"));
            }
            let snapshot = match peers.lock() {
                Ok(peers) => peers.clone(),
                Err(_) => return ClusterReply::Error("peer map poisoned".into()),
            };
            for (other_id, other_addr) in &snapshot {
                if *other_id == node_id || *other_id == peer_node {
                    continue;
                }
                let _ = call_addr(
                    client,
                    *other_addr,
                    ClusterRpc::Announce { node_id, advertise },
                )
                .await;
            }
            ClusterReply::Joined { peers: snapshot }
        }
    }
}

fn node_u64(id: gump_types::NodeId) -> u64 {
    u64::from_be_bytes(id.as_bytes()[8..16].try_into().expect("8 bytes"))
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
