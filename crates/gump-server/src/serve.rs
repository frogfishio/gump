//! Local daemon request handling over an authenticated Unix connection.

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Instant;

use gump_cli::{
    LocalCall, LocalRequest, LocalResponse, MachineOutputV1, PROTOCOL_MAJOR, PROTOCOL_MINOR,
    cancelled_error, deadline_exceeded_error, protocol_mismatch_error, read_frame,
    unauthorized_error, write_frame,
};
use gump_memory::{ControllerAuthority, LeaseTable, RaftCommand};

use crate::framing::FrameError;
use crate::machine::{ErrorBody, StatusBody};
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
    pub memory_cluster: Option<Arc<gump_memory::MemoryCluster>>,
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

    pub fn sync_controller(&mut self, auth: &ControllerAuthority) {
        self.controller_epoch = auth.epoch();
        self.controller_holder = auth.holder();
    }

    pub fn authorize_peer(&self, peer: PeerCred) -> Result<(), PeerAuthError> {
        self.allowlist.authorize(peer)
    }

    fn live_status_fields(&self) -> (u64, Option<u64>, u32, Option<u64>) {
        if let Some(cluster) = &self.memory_cluster {
            if let Ok(snap) = cluster.status_snapshot() {
                return (
                    snap.controller_epoch,
                    snap.controller_holder,
                    snap.voter_count,
                    snap.current_leader,
                );
            }
        }
        (
            self.controller_epoch,
            self.controller_holder,
            self.memory_voters,
            None,
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

fn write_body(
    stream: &mut (impl Read + Write),
    body: LocalResponse,
) -> Result<LocalResponse, ServeError> {
    let out = MachineOutputV1::wrap(body.clone());
    let bytes = serde_json::to_vec(&out).map_err(|e| ServeError::Json(e.to_string()))?;
    write_frame(stream, &bytes)?;
    Ok(body)
}

/// Authenticate peer, negotiate protocol, then serve one request/response exchange.
pub fn serve_connection(
    daemon: &LocalDaemon,
    peer: PeerCred,
    stream: &mut (impl Read + Write),
) -> Result<LocalResponse, ServeError> {
    if daemon.authorize_peer(peer).is_err() {
        return write_body(stream, unauthorized_error());
    }

    let started = Instant::now();
    let payload = read_frame(stream)?;
    let call: LocalCall =
        serde_json::from_slice(&payload).map_err(|e| ServeError::Json(e.to_string()))?;

    if call.protocol_major != PROTOCOL_MAJOR || call.protocol_minor > PROTOCOL_MINOR {
        return write_body(
            stream,
            protocol_mismatch_error(call.protocol_major, call.protocol_minor),
        );
    }

    if call.cancelled {
        return write_body(stream, cancelled_error());
    }
    if call.deadline_ms == Some(0) {
        return write_body(stream, deadline_exceeded_error());
    }

    let body = handle_request(daemon, call.request);

    if let Some(ms) = call.deadline_ms {
        if started.elapsed().as_millis() as u64 > ms {
            return write_body(stream, deadline_exceeded_error());
        }
    }

    write_body(stream, body)
}

pub fn handle_request(daemon: &LocalDaemon, req: LocalRequest) -> LocalResponse {
    let (controller_epoch, controller_holder, memory_voters, leader) = daemon.live_status_fields();
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
        LocalRequest::Observe { subject } => LocalResponse::Observe {
            subject: subject.clone(),
            state: if memory_voters >= 1 {
                "ready".into()
            } else {
                "degraded".into()
            },
            detail: format!(
                "cluster={} voters={} leader={:?}",
                daemon.cluster_id, memory_voters, leader
            ),
        },
        LocalRequest::Deploy {
            operation_id,
            namespace,
            app,
            content_digest_hex,
        } => handle_deploy(daemon, operation_id, namespace, app, content_digest_hex),
        LocalRequest::Lifecycle { action, subject } => {
            let state = match action.as_str() {
                "cancel" | "interrupt" | "wait" => "acknowledged",
                _ => "rejected",
            };
            if state == "rejected" {
                LocalResponse::Error(ErrorBody {
                    code: "INVALID_ARGUMENT".into(),
                    reason: "lifecycle.unknown_action".into(),
                    safe_message: format!("unknown lifecycle action {action:?}"),
                })
            } else {
                LocalResponse::Lifecycle {
                    action,
                    subject,
                    state: state.into(),
                }
            }
        }
        LocalRequest::Recovery { action } => match action.as_str() {
            "status" | "reseal" => LocalResponse::Recovery {
                action,
                sealed: true,
                requires_authority: true,
                detail: "cluster sealed; unseal authority required for new work (N008)".into(),
            },
            _ => LocalResponse::Error(ErrorBody {
                code: "INVALID_ARGUMENT".into(),
                reason: "recovery.unknown_action".into(),
                safe_message: format!("unknown recovery action {action:?}"),
            }),
        },
        LocalRequest::ClusterAdmin { action } => match action.as_str() {
            "members" | "status" => LocalResponse::ClusterAdmin {
                action,
                memory_voters,
                leader,
                detail: if memory_voters == 1 {
                    "one-voter cluster".into()
                } else {
                    format!("{memory_voters} voters")
                },
            },
            _ => LocalResponse::Error(ErrorBody {
                code: "INVALID_ARGUMENT".into(),
                reason: "cluster_admin.unknown_action".into(),
                safe_message: format!("unknown cluster_admin action {action:?}"),
            }),
        },
    }
}

fn handle_deploy(
    daemon: &LocalDaemon,
    operation_id: String,
    namespace: String,
    app: String,
    content_digest_hex: String,
) -> LocalResponse {
    let Some(cluster) = &daemon.memory_cluster else {
        return LocalResponse::Error(ErrorBody {
            code: "UNAVAILABLE".into(),
            reason: "deploy.no_memory_cluster".into(),
            safe_message: "deploy requires a live memory cluster".into(),
        });
    };
    let digest = match parse_blake3_hex(&content_digest_hex) {
        Ok(d) => d,
        Err(msg) => {
            return LocalResponse::Error(ErrorBody {
                code: "INVALID_ARGUMENT".into(),
                reason: "deploy.bad_digest".into(),
                safe_message: msg,
            });
        }
    };
    // Record desired intent through Raft (full upload→execution is GUMP-N010).
    let payload = operation_id.as_bytes().to_vec();
    match cluster.client_write(RaftCommand::PutDesired {
        namespace,
        app,
        expected_generation: 0,
        payload,
        content_digest: digest,
    }) {
        Ok(gump_memory::RaftResponse::Applied(o)) => LocalResponse::Deploy {
            operation_id,
            phase: "intent_recorded".into(),
            reason_code: "deploy.intent_recorded".into(),
            safe_message: "desired intent committed in cluster memory; materialization is N010"
                .into(),
            desired_generation: o.desired_generation,
        },
        Ok(gump_memory::RaftResponse::Rejected(msg)) => LocalResponse::Error(ErrorBody {
            code: "CONFLICT".into(),
            reason: "deploy.rejected".into(),
            safe_message: msg,
        }),
        Ok(other) => LocalResponse::Error(ErrorBody {
            code: "INTERNAL".into(),
            reason: "deploy.unexpected_response".into(),
            safe_message: format!("unexpected raft response {other:?}"),
        }),
        Err(e) => LocalResponse::Error(ErrorBody {
            code: "UNAVAILABLE".into(),
            reason: "deploy.raft_error".into(),
            safe_message: e,
        }),
    }
}

fn parse_blake3_hex(hex: &str) -> Result<[u8; 32], String> {
    if hex.len() != 64 {
        return Err("content digest must be 64 lowercase hex chars".into());
    }
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).map_err(|_| "invalid digest utf8".to_string())?;
        out[i] = u8::from_str_radix(s, 16).map_err(|_| format!("bad hex at byte {i}"))?;
    }
    Ok(out)
}

/// Helper used by tests: acquire controller once and sync into daemon status.
pub fn bootstrap_controller(daemon: &mut LocalDaemon, holder: u64, now_ms: u64) {
    let mut auth = ControllerAuthority::new();
    let mut leases = LeaseTable::default();
    let _ = auth.acquire(holder, now_ms, &mut leases);
    daemon.sync_controller(&auth);
}

pub fn peer_denied_response() -> LocalResponse {
    unauthorized_error()
}
