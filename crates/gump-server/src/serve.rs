//! Local daemon request handling over an authenticated Unix connection.

use std::io::{Read, Write};
use std::sync::Arc;

use gump_memory::{ControllerAuthority, LeaseTable, MemoryCluster};

use crate::framing::{FrameError, read_frame, write_frame};
use crate::machine::{
    ErrorBody, LocalRequest, LocalResponse, MachineOutputV1, StatusBody, unauthorized_error,
};
use crate::peer::{PeerAllowlist, PeerAuthError, PeerCred};

#[derive(Clone, Debug)]
pub struct LocalDaemon {
    pub cluster_id: String,
    pub incarnation: u64,
    pub memory_voters: u32,
    pub allowlist: PeerAllowlist,
    /// Cached controller view; preferred source is [`Self::memory_cluster`] (GUMP-N005).
    pub controller_epoch: u64,
    pub controller_holder: Option<u64>,
    /// Live one-voter Raft node when memory/controller roles are enabled.
    pub memory_cluster: Option<Arc<MemoryCluster>>,
}

impl LocalDaemon {
    pub fn new(allowlist: PeerAllowlist) -> Self {
        Self {
            cluster_id: "local".into(),
            incarnation: 1,
            memory_voters: 1,
            allowlist,
            controller_epoch: 0,
            controller_holder: None,
            memory_cluster: None,
        }
    }

    /// Sync controller fields from an authority record.
    pub fn sync_controller(&mut self, auth: &ControllerAuthority) {
        self.controller_epoch = auth.epoch();
        self.controller_holder = auth.holder();
    }

    pub fn authorize_peer(&self, peer: PeerCred) -> Result<(), PeerAuthError> {
        self.allowlist.authorize(peer)
    }

    /// Prefer live Raft/SM view for status (N005); fall back to cached fields.
    fn live_status_fields(&self) -> (u64, Option<u64>, u32) {
        if let Some(cluster) = &self.memory_cluster {
            if let Ok(snap) = cluster.status_snapshot() {
                return (
                    snap.controller_epoch,
                    snap.controller_holder,
                    snap.voter_count,
                );
            }
        }
        (
            self.controller_epoch,
            self.controller_holder,
            self.memory_voters,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServeError {
    Frame(FrameError),
    Auth(PeerAuthError),
    Json(String),
}

impl std::fmt::Display for ServeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Frame(e) => write!(f, "{e}"),
            Self::Auth(e) => write!(f, "{e}"),
            Self::Json(e) => write!(f, "json: {e}"),
        }
    }
}

impl std::error::Error for ServeError {}

impl From<FrameError> for ServeError {
    fn from(e: FrameError) -> Self {
        Self::Frame(e)
    }
}

impl From<PeerAuthError> for ServeError {
    fn from(e: PeerAuthError) -> Self {
        Self::Auth(e)
    }
}

pub fn handle_request(daemon: &LocalDaemon, req: LocalRequest) -> LocalResponse {
    let (controller_epoch, controller_holder, memory_voters) = daemon.live_status_fields();
    match req {
        LocalRequest::Hello => LocalResponse::Hello {
            daemon: "gump-server".into(),
            controller_epoch,
        },
        LocalRequest::Status => LocalResponse::Status(StatusBody {
            cluster_id: daemon.cluster_id.clone(),
            incarnation: daemon.incarnation,
            controller_epoch,
            controller_holder,
            memory_voters,
            durability_note: if memory_voters <= 1 {
                "1 memory member; live intent has zero failure tolerance".into()
            } else {
                format!("{memory_voters} memory voters; majority required for new commits")
            },
        }),
        LocalRequest::Explain { subject } => LocalResponse::Explain {
            subject,
            reason_code: "status.ok".into(),
            message: "no outstanding explainable fault".into(),
        },
    }
}

/// Authenticate peer, then serve one request/response exchange.
pub fn serve_connection(
    daemon: &LocalDaemon,
    peer: PeerCred,
    stream: &mut (impl Read + Write),
) -> Result<LocalResponse, ServeError> {
    if daemon.authorize_peer(peer).is_err() {
        let body = unauthorized_error();
        let out = MachineOutputV1::wrap(body.clone());
        let bytes = serde_json::to_vec(&out).map_err(|e| ServeError::Json(e.to_string()))?;
        write_frame(stream, &bytes)?;
        return Ok(body);
    }

    let payload = read_frame(stream)?;
    let req: LocalRequest =
        serde_json::from_slice(&payload).map_err(|e| ServeError::Json(e.to_string()))?;
    let body = handle_request(daemon, req);
    let out = MachineOutputV1::wrap(body.clone());
    let bytes = serde_json::to_vec(&out).map_err(|e| ServeError::Json(e.to_string()))?;
    write_frame(stream, &bytes)?;
    Ok(body)
}

/// Helper used by tests: acquire controller once and sync into daemon status.
/// Production `--init` uses [`MemoryCluster`] (GUMP-N005) instead.
pub fn bootstrap_controller(daemon: &mut LocalDaemon, holder: u64, now_ms: u64) {
    let mut auth = ControllerAuthority::new();
    let mut leases = LeaseTable::default();
    let _ = auth.acquire(holder, now_ms, &mut leases);
    daemon.sync_controller(&auth);
}

/// Map auth denial into a stable error body (for goldens without I/O).
pub fn peer_denied_response() -> LocalResponse {
    LocalResponse::Error(ErrorBody {
        code: "UNAUTHORIZED".into(),
        reason: "peer.uid_denied".into(),
        safe_message: "local peer credentials rejected".into(),
    })
}
