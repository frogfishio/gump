//! Authenticated QUIC transport for OpenRaft and ephemeral node enrollment.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use gump_transport::{CaBundle, IdentityMaterial, QuicEndpoint, QuicSession, TransportLimits};
use openraft::error::{InstallSnapshotError, RPCError, RaftError, Unreachable};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::{MemoryNodeId, TypeConfig};
use crate::{RaftCommand, RaftResponse};
use gump_types::Secret;

pub struct ClusterJoinConfig {
    pub seed: SocketAddr,
    pub token: Secret<String>,
}

pub struct ClusterNetworkConfig {
    pub bind: SocketAddr,
    pub advertise: SocketAddr,
    pub material: IdentityMaterial,
    pub trust: CaBundle,
    /// Seed-only allowlist: node id → one-time enrollment token.
    pub join_tokens: BTreeMap<MemoryNodeId, Secret<String>>,
    pub join: Option<ClusterJoinConfig>,
}

#[derive(Clone)]
pub(crate) struct QuicRaftNetworkFactory {
    endpoint: Arc<QuicEndpoint>,
    peers: Arc<Mutex<BTreeMap<MemoryNodeId, SocketAddr>>>,
}

#[derive(Clone)]
pub(crate) struct QuicRaftNetwork {
    endpoint: Arc<QuicEndpoint>,
    target: MemoryNodeId,
    peers: Arc<Mutex<BTreeMap<MemoryNodeId, SocketAddr>>>,
}

#[derive(Serialize, Deserialize)]
pub(crate) enum ClusterRpc {
    Append(AppendEntriesRequest<TypeConfig>),
    Vote(VoteRequest<MemoryNodeId>),
    Install(InstallSnapshotRequest<TypeConfig>),
    ClientWrite(RaftCommand),
    Join {
        node_id: MemoryNodeId,
        advertise: SocketAddr,
        token: String,
    },
    Announce {
        node_id: MemoryNodeId,
        advertise: SocketAddr,
    },
}

#[derive(Serialize, Deserialize)]
pub(crate) enum ClusterReply {
    Append(AppendEntriesResponse<MemoryNodeId>),
    Vote(VoteResponse<MemoryNodeId>),
    Install(InstallSnapshotResponse<MemoryNodeId>),
    ClientWrite(RaftResponse),
    Joined {
        peers: BTreeMap<MemoryNodeId, SocketAddr>,
    },
    Ack,
    Error(String),
}

pub(crate) struct LiveClusterNetwork {
    pub server: Arc<QuicEndpoint>,
    pub client: Arc<QuicEndpoint>,
    pub factory: QuicRaftNetworkFactory,
    pub peers: Arc<Mutex<BTreeMap<MemoryNodeId, SocketAddr>>>,
    pub advertise: SocketAddr,
    pub join_tokens: Arc<Mutex<BTreeMap<MemoryNodeId, [u8; 32]>>>,
    pub used_tokens: Arc<Mutex<BTreeSet<[u8; 32]>>>,
    pub join: Option<ClusterJoinConfig>,
}

impl LiveClusterNetwork {
    pub(crate) fn build(config: ClusterNetworkConfig) -> Result<Self, String> {
        let limits = TransportLimits::default();
        let local_node = node_u64(config.material.identity.node_id);
        let server = Arc::new(
            QuicEndpoint::server(&config.material, &config.trust, config.bind, limits)
                .map_err(|e| e.to_string())?,
        );
        let client_bind = SocketAddr::new(
            match config.bind.ip() {
                IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                IpAddr::V6(_) => "::".parse().expect("IPv6 unspecified"),
            },
            0,
        );
        let client = Arc::new(
            QuicEndpoint::client(&config.material, &config.trust, client_bind, limits)
                .map_err(|e| e.to_string())?,
        );
        let mut initial_peers = BTreeMap::new();
        initial_peers.insert(local_node, config.advertise);
        let peers = Arc::new(Mutex::new(initial_peers));
        let factory = QuicRaftNetworkFactory {
            endpoint: Arc::clone(&client),
            peers: Arc::clone(&peers),
        };
        let join_tokens = config
            .join_tokens
            .into_iter()
            .map(|(node, token)| (node, *blake3::hash(token.expose().as_bytes()).as_bytes()))
            .collect();
        Ok(Self {
            server,
            client,
            factory,
            peers,
            advertise: config.advertise,
            join_tokens: Arc::new(Mutex::new(join_tokens)),
            used_tokens: Arc::new(Mutex::new(BTreeSet::new())),
            join: config.join,
        })
    }
}

fn node_u64(id: gump_types::NodeId) -> u64 {
    u64::from_be_bytes(id.as_bytes()[8..16].try_into().expect("8 bytes"))
}

impl RaftNetworkFactory<TypeConfig> for QuicRaftNetworkFactory {
    type Network = QuicRaftNetwork;

    async fn new_client(&mut self, target: MemoryNodeId, _node: &()) -> Self::Network {
        QuicRaftNetwork {
            endpoint: Arc::clone(&self.endpoint),
            target,
            peers: Arc::clone(&self.peers),
        }
    }
}

impl QuicRaftNetwork {
    async fn call(&self, request: ClusterRpc) -> Result<ClusterReply, io::Error> {
        let addr = self
            .peers
            .lock()
            .map_err(|_| io::Error::other("peer map poisoned"))?
            .get(&self.target)
            .copied()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "unknown Raft peer"))?;
        call_addr(&self.endpoint, addr, request).await
    }
}

pub(crate) async fn call_addr(
    endpoint: &QuicEndpoint,
    addr: SocketAddr,
    mut request: ClusterRpc,
) -> Result<ClusterReply, io::Error> {
    let session = endpoint.connect(addr).await.map_err(io_other)?;
    let (mut send, mut recv) = session.open_bi().await.map_err(io_other)?;
    let mut body = serde_json::to_vec(&request).map_err(io_other)?;
    if let ClusterRpc::Join { token, .. } = &mut request {
        token.zeroize();
    }
    let sent = session.send_control(&mut send, &body).await;
    body.zeroize();
    sent.map_err(io_other)?;
    send.finish().map_err(io_other)?;
    let response = session.recv_control(&mut recv).await.map_err(io_other)?;
    serde_json::from_slice(&response).map_err(io_other)
}

pub(crate) async fn reply(
    session: &QuicSession,
    mut send: quinn::SendStream,
    reply: ClusterReply,
) -> Result<(), String> {
    let body = serde_json::to_vec(&reply).map_err(|e| e.to_string())?;
    session
        .send_control(&mut send, &body)
        .await
        .map_err(|e| e.to_string())?;
    send.finish().map_err(|e| e.to_string())?;
    send.stopped().await.map_err(|e| e.to_string())?;
    Ok(())
}

fn io_other(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

fn unreachable<E>(e: io::Error) -> RPCError<MemoryNodeId, (), E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    RPCError::Unreachable(Unreachable::new(&e))
}

impl RaftNetwork<TypeConfig> for QuicRaftNetwork {
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<
        AppendEntriesResponse<MemoryNodeId>,
        RPCError<MemoryNodeId, (), RaftError<MemoryNodeId>>,
    > {
        match self.call(ClusterRpc::Append(rpc)).await {
            Ok(ClusterReply::Append(r)) => Ok(r),
            Ok(ClusterReply::Error(e)) => Err(unreachable(io::Error::other(e))),
            Ok(_) => Err(unreachable(io::Error::other("wrong append reply"))),
            Err(e) => Err(unreachable(e)),
        }
    }

    async fn install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<
        InstallSnapshotResponse<MemoryNodeId>,
        RPCError<MemoryNodeId, (), RaftError<MemoryNodeId, InstallSnapshotError>>,
    > {
        match self.call(ClusterRpc::Install(rpc)).await {
            Ok(ClusterReply::Install(r)) => Ok(r),
            Ok(ClusterReply::Error(e)) => Err(unreachable(io::Error::other(e))),
            Ok(_) => Err(unreachable(io::Error::other("wrong snapshot reply"))),
            Err(e) => Err(unreachable(e)),
        }
    }

    async fn vote(
        &mut self,
        rpc: VoteRequest<MemoryNodeId>,
        _option: RPCOption,
    ) -> Result<VoteResponse<MemoryNodeId>, RPCError<MemoryNodeId, (), RaftError<MemoryNodeId>>>
    {
        match self.call(ClusterRpc::Vote(rpc)).await {
            Ok(ClusterReply::Vote(r)) => Ok(r),
            Ok(ClusterReply::Error(e)) => Err(unreachable(io::Error::other(e))),
            Ok(_) => Err(unreachable(io::Error::other("wrong vote reply"))),
            Err(e) => Err(unreachable(e)),
        }
    }
}
