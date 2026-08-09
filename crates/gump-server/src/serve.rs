//! Local daemon request handling over an authenticated Unix connection.

use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use gump_cli::{
    InventoryEntryBody, LocalCall, LocalRequest, LocalResponse, MachineOutputV1, PROTOCOL_MAJOR,
    PROTOCOL_MINOR, TelemetryEventBody, cancelled_error, deadline_exceeded_error,
    intent_accepted_stages, protocol_mismatch_error, read_frame, unauthorized_error, wait_body,
    write_frame,
};
use gump_connectors::{ObjectStore, OrphanCapsule, RuntimeObjectStore, parse_final_capsule_key};
use gump_crypto::SignerTrustPolicy;
use gump_memory::{ControllerAuthority, LeaseTable};
use gump_telemetry::{RingConfig, TelemetryEventView, TelemetryPlane};

use crate::custody::ClusterCustody;
use crate::deploy_txn::{
    DeployTxnOutcome, DeployTxnRequest, capsule_identity_reader, decode_capsule_hex,
    parse_cluster_id, parse_operation_id, run_verified_deploy_reader,
};
use crate::framing::FrameError;
use crate::machine::{ErrorBody, StatusBody};
use crate::peer::{PeerAllowlist, PeerAuthError, PeerCred};
use crate::recovery_txn::{
    ReintroduceOutcome, ReintroduceRequest, digest_hex, run_reintroduce, verify_stored_capsule,
};
use crate::runtime::RuntimeCoordinator;

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
    /// In-memory unseal custody (GUMP-N008). Absent when custody role is off.
    pub custody: Option<Arc<Mutex<ClusterCustody>>>,
    /// Capsule object store for deploy publish (GUMP-N010). Production is S3;
    /// memory mode must be selected explicitly for local tests.
    pub object_store: Option<Arc<Mutex<RuntimeObjectStore>>>,
    /// Explicit release-signer enrollment. Embedded Capsule keys never enroll
    /// themselves.
    pub signer_trust: Arc<SignerTrustPolicy>,
    /// Inert Capsules left after publish-without-intent (PROTOCOL.md §13).
    pub deploy_orphans: Arc<Mutex<Vec<OrphanCapsule>>>,
    /// Memory-only recent-window telemetry plane (GUMP-N014). Absent when facet off.
    pub telemetry: Option<Arc<Mutex<TelemetryPlane>>>,
    /// Shared execution loop/status; absent on non-controller/agent nodes.
    pub execution: Option<Arc<Mutex<RuntimeCoordinator>>>,
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
            custody: None,
            object_store: None,
            signer_trust: Arc::new(SignerTrustPolicy::new()),
            deploy_orphans: Arc::new(Mutex::new(Vec::new())),
            telemetry: None,
            execution: None,
        }
    }

    pub fn with_telemetry_plane(mut self, plane: TelemetryPlane) -> Self {
        self.telemetry = Some(Arc::new(Mutex::new(plane)));
        self
    }

    pub fn enable_default_telemetry(&mut self, ring_capacity_bytes: usize) {
        self.telemetry = Some(Arc::new(Mutex::new(TelemetryPlane::new(RingConfig {
            max_bytes: ring_capacity_bytes.max(1),
            ..RingConfig::default()
        }))));
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
        LocalRequest::Explain { subject } => {
            handle_explain(daemon, subject, controller_epoch, memory_voters, leader)
        }
        LocalRequest::Observe { subject } => handle_observe(daemon, subject, memory_voters, leader),
        LocalRequest::Deploy {
            operation_id,
            namespace,
            app,
            content_digest_hex,
            capsule_hex,
            capsule_path,
            wait,
        } => handle_deploy(
            daemon,
            operation_id,
            namespace,
            app,
            content_digest_hex,
            capsule_hex,
            capsule_path,
            wait,
        ),
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
                let note = match action.as_str() {
                    "interrupt" | "cancel" => Some(
                        "acknowledged; does not roll back published Capsules or committed intent"
                            .into(),
                    ),
                    "wait" => Some(
                        "wait observes declared conditions only; loss of observation ≠ rollback"
                            .into(),
                    ),
                    _ => None,
                };
                LocalResponse::Lifecycle {
                    action,
                    subject,
                    state: state.into(),
                    interrupted_implies_rollback: false,
                    note,
                }
            }
        }
        LocalRequest::Recovery {
            action,
            provider,
            key_id,
            recovery_secret_hex,
        } => handle_recovery(daemon, action, provider, key_id, recovery_secret_hex),
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
        LocalRequest::Telemetry { filter, max_events } => {
            handle_telemetry(daemon, filter, max_events)
        }
        LocalRequest::Inventory => handle_inventory(daemon),
        LocalRequest::Inspect { capsule_id } => handle_inspect(daemon, capsule_id),
        LocalRequest::Reintroduce {
            capsule_id,
            plan,
            finite_mode,
            resume_from,
            operation_id,
            namespace,
            app,
        } => handle_reintroduce(
            daemon,
            ReintroduceArgs {
                capsule_id,
                plan,
                finite_mode,
                resume_from,
                operation_id,
                namespace,
                app,
            },
        ),
    }
}

fn handle_observe(
    daemon: &LocalDaemon,
    subject: String,
    memory_voters: u32,
    leader: Option<u64>,
) -> LocalResponse {
    let Some(execution) = &daemon.execution else {
        return LocalResponse::Observe {
            subject,
            state: "disabled".into(),
            detail: "this node has no composed execution loop".into(),
        };
    };
    let Ok(execution) = execution.lock() else {
        return LocalResponse::Observe {
            subject,
            state: "degraded".into(),
            detail: "execution status lock poisoned".into(),
        };
    };
    let status = execution.status();
    let state = if status.last_error.is_some() {
        "degraded"
    } else if status.placements > 0 {
        "running"
    } else if status.completed > 0 {
        "completed"
    } else if status.desired > 0 {
        "pending"
    } else {
        "idle"
    };
    LocalResponse::Observe {
        subject,
        state: state.into(),
        detail: format!(
            "cluster={} voters={} leader={leader:?} desired={} local_placements={} ready={} completed_units={} hiccup_presence={} ringtail_active={} ringtail_accepted={} ringtail_failed={} ringtail_dropped={} last_error={}",
            daemon.cluster_id,
            memory_voters,
            status.desired,
            status.placements,
            status.ready,
            status.completed,
            status.hiccup_presence,
            status.ringtail_active,
            status.ringtail_accepted,
            status.ringtail_failed,
            status.ringtail_dropped,
            status.last_error.as_deref().unwrap_or("none")
        ),
    }
}

fn handle_telemetry(
    daemon: &LocalDaemon,
    filter: Option<String>,
    max_events: Option<u32>,
) -> LocalResponse {
    let Some(plane) = &daemon.telemetry else {
        return LocalResponse::Error(ErrorBody {
            code: "FAILED_PRECONDITION".into(),
            reason: "telemetry.disabled".into(),
            safe_message: "telemetry facet is not enabled on this node".into(),
        });
    };
    let Ok(guard) = plane.lock() else {
        return LocalResponse::Error(ErrorBody {
            code: "INTERNAL".into(),
            reason: "telemetry.lock_poisoned".into(),
            safe_message: "telemetry plane unavailable".into(),
        });
    };
    let max = max_events.unwrap_or(256) as usize;
    let snap = guard.query(filter.as_deref(), max);
    LocalResponse::Telemetry {
        profile: snap.profile.into(),
        memory_only: snap.memory_only,
        pushed: snap.pushed,
        dropped_oldest: snap.dropped_oldest,
        filter: snap.filter,
        caught_up: snap.caught_up,
        identity_note: snap.identity_note.into(),
        events: snap
            .events
            .into_iter()
            .map(|e| match e {
                TelemetryEventView::Record {
                    topic,
                    stream_sequence,
                    utf8_hint,
                    bytes_hex,
                    text,
                } => TelemetryEventBody::Record {
                    topic,
                    stream_sequence,
                    utf8_hint,
                    bytes_hex,
                    text,
                },
                TelemetryEventView::Gap {
                    topic,
                    from_sequence,
                    to_sequence,
                    reason,
                } => TelemetryEventBody::Gap {
                    topic,
                    from_sequence,
                    to_sequence,
                    reason: reason.into(),
                },
            })
            .collect(),
    }
}

fn handle_recovery(
    daemon: &LocalDaemon,
    action: String,
    provider: Option<String>,
    key_id: Option<String>,
    recovery_secret_hex: Option<String>,
) -> LocalResponse {
    let Some(custody) = &daemon.custody else {
        return LocalResponse::Error(ErrorBody {
            code: "UNAVAILABLE".into(),
            reason: "recovery.no_custody".into(),
            safe_message: "custody facet not enabled on this node".into(),
        });
    };
    let Ok(mut guard) = custody.lock() else {
        return LocalResponse::Error(ErrorBody {
            code: "INTERNAL".into(),
            reason: "recovery.custody_poisoned".into(),
            safe_message: "custody lock poisoned".into(),
        });
    };
    match action.as_str() {
        "status" => {
            let st = guard.status();
            LocalResponse::Recovery {
                action,
                sealed: st.sealed,
                requires_authority: st.requires_authority,
                detail: recovery_detail(&st),
                cluster_public_key_hex: guard.cluster_public().map(|k| hex_lower(&k)),
                key_id: st.key_id.clone(),
            }
        }
        "reseal" => {
            let st = guard.reseal();
            LocalResponse::Recovery {
                action,
                sealed: st.sealed,
                requires_authority: st.requires_authority,
                detail: recovery_detail(&st),
                cluster_public_key_hex: None,
                key_id: None,
            }
        }
        "unseal" => match activate_custody(
            &mut guard,
            provider.as_deref(),
            key_id.as_deref(),
            recovery_secret_hex.as_deref(),
        ) {
            Ok(st) => LocalResponse::Recovery {
                action,
                sealed: st.sealed,
                requires_authority: st.requires_authority,
                detail: recovery_detail(&st),
                cluster_public_key_hex: guard.cluster_public().map(|k| hex_lower(&k)),
                key_id: st.key_id.clone(),
            },
            Err(e) => LocalResponse::Error(ErrorBody {
                code: "FAILED_PRECONDITION".into(),
                reason: "recovery.unseal_failed".into(),
                safe_message: e,
            }),
        },
        _ => LocalResponse::Error(ErrorBody {
            code: "INVALID_ARGUMENT".into(),
            reason: "recovery.unknown_action".into(),
            safe_message: format!("unknown recovery action {action:?}"),
        }),
    }
}

fn recovery_detail(st: &crate::custody::CustodyStatus) -> String {
    if st.sealed {
        "cluster sealed; unseal authority required for new work".into()
    } else {
        format!(
            "cluster active via {} key_id={}",
            st.provider_type.as_deref().unwrap_or("unknown"),
            st.key_id.as_deref().unwrap_or("unknown")
        )
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn activate_custody(
    custody: &mut ClusterCustody,
    provider: Option<&str>,
    key_id: Option<&str>,
    recovery_secret_hex: Option<&str>,
) -> Result<crate::custody::CustodyStatus, String> {
    let provider = provider.unwrap_or("software");
    let key_id = key_id.unwrap_or("default");
    match provider {
        "software" => {
            let hex = recovery_secret_hex.ok_or_else(|| {
                "software unseal requires recovery_secret_hex (32-byte hex)".to_string()
            })?;
            let secret = parse_recovery_secret_hex(hex)?;
            custody
                .activate_software_1of1(&secret, key_id)
                .map_err(|e| e.to_string())
        }
        // Fake HSM is activated in-process via [`ClusterCustody::activate_fake_hsm`]
        // (conformance / tests); the local API does not mint HSM keys over JSON.
        "fake-hsm" => Err(
            "fake-hsm unseal is not available via recovery API; use in-process activation".into(),
        ),
        other => Err(format!("unknown unseal provider {other:?}")),
    }
}

fn parse_recovery_secret_hex(hex: &str) -> Result<gump_crypto::RecoverySecret, String> {
    if hex.len() != 64 {
        return Err("recovery secret must be 64 lowercase hex chars".into());
    }
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).map_err(|_| "invalid secret utf8".to_string())?;
        out[i] = u8::from_str_radix(s, 16).map_err(|_| "bad recovery secret hex".to_string())?;
    }
    Ok(gump_crypto::RecoverySecret::from_bytes(out))
}

fn handle_explain(
    daemon: &LocalDaemon,
    subject: String,
    controller_epoch: u64,
    memory_voters: u32,
    leader: Option<u64>,
) -> LocalResponse {
    let durability_note = if memory_voters <= 1 {
        "1 memory member; live intent has zero failure tolerance".into()
    } else {
        format!("{memory_voters} memory voters; majority required for new commits")
    };
    // Explain reads committed/observed cluster view only; detail may be compacted.
    let compaction_disclosed = memory_voters <= 1 || daemon.memory_cluster.is_none();
    let (reason_code, message, observation_source) = if daemon.memory_cluster.is_some() {
        (
            "status.ok".into(),
            format!(
                "subject={subject}; controller_epoch={controller_epoch}; voters={memory_voters}; leader={leader:?}; no outstanding explainable fault in committed view"
            ),
            "committed_cluster_memory".into(),
        )
    } else {
        (
            "status.degraded".into(),
            format!(
                "subject={subject}; no live memory cluster; observation limited to local daemon fields"
            ),
            "observed".into(),
        )
    };
    LocalResponse::Explain {
        subject,
        reason_code,
        message,
        observation_source,
        compaction_disclosed,
        durability_note,
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_deploy(
    daemon: &LocalDaemon,
    operation_id: String,
    namespace: String,
    app: String,
    content_digest_hex: String,
    capsule_hex: Option<String>,
    capsule_path: Option<String>,
    wait: Option<String>,
) -> LocalResponse {
    if let Some(custody) = &daemon.custody {
        let sealed = custody.lock().map(|g| g.is_sealed()).unwrap_or(true);
        if sealed {
            return LocalResponse::Error(ErrorBody {
                code: "FAILED_PRECONDITION".into(),
                reason: "deploy.custody_sealed".into(),
                safe_message: "cluster sealed; unseal authority required for new work".into(),
            });
        }
    }
    let Some(cluster) = &daemon.memory_cluster else {
        return LocalResponse::Error(ErrorBody {
            code: "UNAVAILABLE".into(),
            reason: "deploy.no_memory_cluster".into(),
            safe_message: "deploy requires a live memory cluster".into(),
        });
    };
    let Some(store) = &daemon.object_store else {
        return LocalResponse::Error(ErrorBody {
            code: "UNAVAILABLE".into(),
            reason: "deploy.no_object_store".into(),
            safe_message: "deploy requires an object-store connector".into(),
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
    let op_bytes = match parse_operation_id(&operation_id) {
        Ok(b) => b,
        Err(msg) => {
            return LocalResponse::Error(ErrorBody {
                code: "INVALID_ARGUMENT".into(),
                reason: "deploy.bad_operation_id".into(),
                safe_message: msg,
            });
        }
    };
    let capsule_bytes = match decode_capsule_hex(capsule_hex.as_deref(), &digest) {
        Ok(b) => b,
        Err(msg) => {
            return LocalResponse::Error(ErrorBody {
                code: "INVALID_ARGUMENT".into(),
                reason: "deploy.bad_capsule".into(),
                safe_message: msg,
            });
        }
    };

    let mut source: Box<dyn ReadSeek> = match (capsule_bytes, capsule_path) {
        (Some(bytes), None) => Box::new(std::io::Cursor::new(bytes)),
        (None, Some(path)) => match open_capsule_file(&path) {
            Ok(file) => Box::new(file),
            Err(msg) => {
                return LocalResponse::Error(ErrorBody {
                    code: "INVALID_ARGUMENT".into(),
                    reason: "deploy.capsule_open".into(),
                    safe_message: msg,
                });
            }
        },
        _ => {
            return LocalResponse::Error(ErrorBody {
                code: "INVALID_ARGUMENT".into(),
                reason: "deploy.capsule_required".into(),
                safe_message: "deploy requires exactly one sealed Capsule source".into(),
            });
        }
    };
    let content_length = match source.seek(SeekFrom::End(0)) {
        Ok(n) if n > 0 => n,
        Ok(_) => {
            return LocalResponse::Error(ErrorBody {
                code: "INVALID_ARGUMENT".into(),
                reason: "deploy.empty_capsule".into(),
                safe_message: "Capsule file is empty".into(),
            });
        }
        Err(e) => {
            return LocalResponse::Error(ErrorBody {
                code: "INVALID_ARGUMENT".into(),
                reason: "deploy.capsule_seek".into(),
                safe_message: format!("cannot inspect Capsule length: {e}"),
            });
        }
    };
    let _ = source.seek(SeekFrom::Start(0));
    let (capsule_cluster, capsule_id) = match capsule_identity_reader(&mut source) {
        Ok(ids) => ids,
        Err(msg) => {
            return LocalResponse::Error(ErrorBody {
                code: "INVALID_ARGUMENT".into(),
                reason: "deploy.bad_capsule".into(),
                safe_message: msg,
            });
        }
    };
    if let Err(e) = source.seek(SeekFrom::Start(0)) {
        return LocalResponse::Error(ErrorBody {
            code: "INVALID_ARGUMENT".into(),
            reason: "deploy.capsule_seek".into(),
            safe_message: format!("cannot rewind verified Capsule: {e}"),
        });
    }
    let daemon_cluster = parse_cluster_id(&daemon.cluster_id);
    if capsule_cluster != daemon_cluster {
        return LocalResponse::Error(ErrorBody {
            code: "FAILED_PRECONDITION".into(),
            reason: "deploy.cluster_mismatch".into(),
            safe_message: "Capsule was sealed for a different cluster".into(),
        });
    }

    let expected_generation = match cluster.desired_generation(&namespace, &app) {
        Ok(generation) => generation,
        Err(message) => {
            return LocalResponse::Error(ErrorBody {
                code: "UNAVAILABLE".into(),
                reason: "deploy.desired_generation".into(),
                safe_message: message,
            });
        }
    };

    let req = DeployTxnRequest {
        operation_id: op_bytes,
        operation_id_display: operation_id.clone(),
        namespace,
        app,
        expected_generation,
        content_digest: digest,
        capsule_bytes: None,
        cluster_id: daemon_cluster,
        capsule_id,
    };

    let mut store_guard = match store.lock() {
        Ok(g) => g,
        Err(_) => {
            return LocalResponse::Error(ErrorBody {
                code: "INTERNAL".into(),
                reason: "deploy.store_lock".into(),
                safe_message: "object store lock poisoned".into(),
            });
        }
    };
    let mut orphans_guard = match daemon.deploy_orphans.lock() {
        Ok(g) => g,
        Err(_) => {
            return LocalResponse::Error(ErrorBody {
                code: "INTERNAL".into(),
                reason: "deploy.orphan_lock".into(),
                safe_message: "orphan list lock poisoned".into(),
            });
        }
    };

    let (_, _, memory_voters, _) = daemon.live_status_fields();
    let durability_note = if memory_voters <= 1 {
        "1 memory member; live intent has zero failure tolerance".into()
    } else {
        format!("{memory_voters} memory voters; majority required for new commits")
    };
    let wait_info = wait_body(wait.as_deref());

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    match run_verified_deploy_reader(
        &mut *store_guard,
        &daemon.signer_trust,
        cluster,
        &mut orphans_guard,
        req,
        now_ms,
        content_length,
        &mut source,
    ) {
        DeployTxnOutcome::Success {
            desired_generation,
            replayed,
            ..
        } => LocalResponse::Deploy {
            operation_id,
            phase: if replayed {
                "intent_accepted_replay".into()
            } else {
                "intent_accepted".into()
            },
            reason_code: if replayed {
                "deploy.intent_accepted_replay".into()
            } else {
                "deploy.intent_accepted".into()
            },
            safe_message: if replayed {
                "replayed committed deploy receipt from cluster memory (Raft Idempotent); later stages remain observed separately".into()
            } else {
                "upload→publish→intent committed; scheduling/start/readiness/publication/completion remain observed separately".into()
            },
            desired_generation: Some(desired_generation),
            content_digest_hex,
            durability_note,
            wait: wait_info,
            stages: intent_accepted_stages(replayed),
            interrupted_implies_rollback: false,
        },
        DeployTxnOutcome::Conflict { .. } => LocalResponse::Error(ErrorBody {
            code: "CONFLICT".into(),
            reason: "deploy.idempotency_conflict".into(),
            safe_message: "same operation_id with a different request digest".into(),
        }),
        DeployTxnOutcome::Failed {
            phase,
            reason,
            orphan,
        } => {
            let code = if orphan.is_some() {
                "FAILED_PRECONDITION"
            } else {
                "INVALID_ARGUMENT"
            };
            LocalResponse::Error(ErrorBody {
                code: code.into(),
                reason: format!("deploy.{}", phase.as_str()),
                safe_message: if orphan.is_some() {
                    format!("{reason}; inert orphan Capsule reported")
                } else {
                    reason
                },
            })
        }
    }
}

trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

fn open_capsule_file(path: &str) -> Result<std::fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| format!("open Capsule: {e}"))?;
    let metadata = file
        .metadata()
        .map_err(|e| format!("inspect Capsule: {e}"))?;
    if !metadata.is_file() {
        return Err("Capsule source must be a regular file".into());
    }
    Ok(file)
}

fn handle_inventory(daemon: &LocalDaemon) -> LocalResponse {
    let Some(store) = &daemon.object_store else {
        return LocalResponse::Error(ErrorBody {
            code: "UNAVAILABLE".into(),
            reason: "inventory.no_object_store".into(),
            safe_message: "inventory requires an object-store connector".into(),
        });
    };
    let store_guard = match store.lock() {
        Ok(g) => g,
        Err(_) => {
            return LocalResponse::Error(ErrorBody {
                code: "INTERNAL".into(),
                reason: "inventory.store_lock".into(),
                safe_message: "object store lock poisoned".into(),
            });
        }
    };
    let desired_count = daemon
        .memory_cluster
        .as_ref()
        .and_then(|c| c.desired_len().ok())
        .unwrap_or(0) as u64;
    let mut capsules = Vec::new();
    let evidence = match ObjectStore::list_final_capsules(&*store_guard, 10_000) {
        Ok(items) => items,
        Err(e) => {
            return LocalResponse::Error(ErrorBody {
                code: "UNAVAILABLE".into(),
                reason: "inventory.object_store".into(),
                safe_message: format!("object-store inventory failed: {e}"),
            });
        }
    };
    for ev in evidence {
        let Some((_, capsule)) = parse_final_capsule_key(&ev.key) else {
            continue;
        };
        let live_referenced = daemon
            .memory_cluster
            .as_ref()
            .and_then(|c| c.desired_references_digest(&ev.digest).ok())
            .unwrap_or(false);
        capsules.push(InventoryEntryBody {
            capsule_id: capsule.to_hyphenated(),
            content_digest_hex: digest_hex(&ev.digest),
            size_bytes: ev.length,
            object_key: ev.key.as_str().to_string(),
            live_referenced,
            inert: true,
        });
    }
    capsules.sort_by(|a, b| a.capsule_id.cmp(&b.capsule_id));
    LocalResponse::Inventory {
        desired_count,
        note: "inert Capsules from object store; unreferenced after full loss ≠ obsolete or safe to delete".into(),
        capsules,
    }
}

fn handle_inspect(daemon: &LocalDaemon, capsule_id: String) -> LocalResponse {
    let Some(store) = &daemon.object_store else {
        return LocalResponse::Error(ErrorBody {
            code: "UNAVAILABLE".into(),
            reason: "inspect.no_object_store".into(),
            safe_message: "inspect requires an object-store connector".into(),
        });
    };
    let store_guard = match store.lock() {
        Ok(g) => g,
        Err(_) => {
            return LocalResponse::Error(ErrorBody {
                code: "INTERNAL".into(),
                reason: "inspect.store_lock".into(),
                safe_message: "object store lock poisoned".into(),
            });
        }
    };
    match verify_stored_capsule(&*store_guard, &daemon.cluster_id, &capsule_id) {
        Ok(c) => {
            let live_referenced = daemon
                .memory_cluster
                .as_ref()
                .and_then(|m| m.desired_references_digest(&c.content_digest).ok())
                .unwrap_or(false);
            LocalResponse::Inspect {
                capsule_id: c.capsule_id.to_hyphenated(),
                content_digest_hex: digest_hex(&c.content_digest),
                size_bytes: c.size_bytes,
                object_key: c.object_key,
                live_referenced,
                public_note: "public metadata only; protected values never printed".into(),
            }
        }
        Err(msg) => LocalResponse::Error(ErrorBody {
            code: "NOT_FOUND".into(),
            reason: "inspect.not_found".into(),
            safe_message: msg,
        }),
    }
}

struct ReintroduceArgs {
    capsule_id: String,
    plan: bool,
    finite_mode: Option<String>,
    resume_from: Option<String>,
    operation_id: Option<String>,
    namespace: Option<String>,
    app: Option<String>,
}

fn handle_reintroduce(daemon: &LocalDaemon, args: ReintroduceArgs) -> LocalResponse {
    if let Some(custody) = &daemon.custody {
        let sealed = custody.lock().map(|g| g.is_sealed()).unwrap_or(true);
        if sealed && !args.plan {
            return LocalResponse::Error(ErrorBody {
                code: "FAILED_PRECONDITION".into(),
                reason: "reintroduce.custody_sealed".into(),
                safe_message: "cluster sealed; unseal authority required for new work".into(),
            });
        }
    }
    let Some(cluster) = &daemon.memory_cluster else {
        return LocalResponse::Error(ErrorBody {
            code: "UNAVAILABLE".into(),
            reason: "reintroduce.no_memory_cluster".into(),
            safe_message: "reintroduce requires a live memory cluster".into(),
        });
    };
    let Some(store) = &daemon.object_store else {
        return LocalResponse::Error(ErrorBody {
            code: "UNAVAILABLE".into(),
            reason: "reintroduce.no_object_store".into(),
            safe_message: "reintroduce requires an object-store connector".into(),
        });
    };
    let store_guard = match store.lock() {
        Ok(g) => g,
        Err(_) => {
            return LocalResponse::Error(ErrorBody {
                code: "INTERNAL".into(),
                reason: "reintroduce.store_lock".into(),
                safe_message: "object store lock poisoned".into(),
            });
        }
    };
    let ns = args.namespace.unwrap_or_else(|| "default".into());
    let app_name = args.app.unwrap_or_else(|| "app".into());
    let (_, _, memory_voters, _) = daemon.live_status_fields();
    let durability_note = if memory_voters <= 1 {
        "1 memory member; live intent has zero failure tolerance".into()
    } else {
        format!("{memory_voters} memory voters; majority required for new commits")
    };

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    match run_reintroduce(
        &*store_guard,
        &daemon.signer_trust,
        now_ms,
        cluster,
        &ReintroduceRequest {
            cluster_id: &daemon.cluster_id,
            capsule_id: &args.capsule_id,
            plan: args.plan,
            finite_mode: args.finite_mode.as_deref(),
            resume_from: args.resume_from.as_deref(),
            operation_id: args.operation_id.as_deref(),
            namespace: &ns,
            app: &app_name,
        },
    ) {
        ReintroduceOutcome::Plan { capsule, finite_mode } => LocalResponse::Reintroduce {
            capsule_id: capsule.capsule_id.to_hyphenated(),
            plan: true,
            phase: "plan_ready".into(),
            reason_code: "reintroduce.plan".into(),
            safe_message: "verified inert Capsule; proposed fresh intent without mutating K/V"
                .into(),
            content_digest_hex: digest_hex(&capsule.content_digest),
            finite_mode: Some(finite_mode).filter(|s| s != "unspecified_plan"),
            desired_generation: None,
            durability_note,
            restores_prior_desired: false,
        },
        ReintroduceOutcome::Applied {
            capsule,
            finite_mode,
            desired_generation,
            replayed,
        } => LocalResponse::Reintroduce {
            capsule_id: capsule.capsule_id.to_hyphenated(),
            plan: false,
            phase: if replayed {
                "intent_accepted_replay".into()
            } else {
                "intent_accepted".into()
            },
            reason_code: if replayed {
                "reintroduce.intent_accepted_replay".into()
            } else {
                "reintroduce.intent_accepted".into()
            },
            safe_message: "fresh intent accepted for selected Capsule; prior desired/completion not restored from S3".into(),
            content_digest_hex: digest_hex(&capsule.content_digest),
            finite_mode: Some(finite_mode),
            desired_generation: Some(desired_generation),
            durability_note,
            restores_prior_desired: false,
        },
        ReintroduceOutcome::Failed { reason, code } => LocalResponse::Error(ErrorBody {
            code: code.into(),
            reason: "reintroduce.failed".into(),
            safe_message: reason,
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

#[cfg(test)]
mod n008_tests {
    use super::*;
    use crate::custody::ClusterCustody;
    use gump_memory::MemoryCluster;

    fn daemon_with_custody() -> LocalDaemon {
        let cluster_id = [
            0x01, 0x8f, 0x4a, 0x00, 0x00, 0x00, 0x70, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x55,
        ];
        let mut d = LocalDaemon::new(PeerAllowlist::same_uid(1));
        d.cluster_id = "n008".into();
        d.memory_cluster = Some(Arc::new(
            MemoryCluster::bootstrap_one_voter(1, 1).expect("raft"),
        ));
        d.custody = Some(Arc::new(Mutex::new(ClusterCustody::new_sealed(cluster_id))));
        d.object_store = Some(Arc::new(Mutex::new(RuntimeObjectStore::Memory(
            gump_connectors::FakeObjectStore::new(),
        ))));
        d
    }

    fn to_hex(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    #[test]
    fn deploy_fails_while_sealed_then_rejects_unverified_bytes_after_unseal() {
        let daemon = daemon_with_custody();
        let body = b"n008-capsule";
        let digest = to_hex(blake3::hash(body).as_bytes());
        let capsule_hex = to_hex(body);
        let sealed_err = handle_request(
            &daemon,
            LocalRequest::Deploy {
                operation_id: "op1".into(),
                namespace: "ns".into(),
                app: "app".into(),
                content_digest_hex: digest.clone(),
                capsule_hex: Some(capsule_hex.clone()),
                capsule_path: None,
                wait: None,
            },
        );
        match sealed_err {
            LocalResponse::Error(e) => {
                assert_eq!(e.reason, "deploy.custody_sealed");
                assert!(!e.safe_message.contains("11".repeat(32).as_str()));
            }
            other => panic!("expected sealed deploy error, got {other:?}"),
        }

        let unsealed = handle_request(
            &daemon,
            LocalRequest::Recovery {
                action: "unseal".into(),
                provider: Some("software".into()),
                key_id: Some("soft-1".into()),
                recovery_secret_hex: Some("11".repeat(32)),
            },
        );
        match unsealed {
            LocalResponse::Recovery {
                sealed,
                requires_authority,
                ..
            } => {
                assert!(!sealed);
                assert!(!requires_authority);
            }
            other => panic!("expected recovery ok, got {other:?}"),
        }

        let deployed = handle_request(
            &daemon,
            LocalRequest::Deploy {
                operation_id: "op2".into(),
                namespace: "ns".into(),
                app: "app".into(),
                content_digest_hex: digest,
                capsule_hex: Some(capsule_hex),
                capsule_path: None,
                wait: None,
            },
        );
        assert!(matches!(
            deployed,
            LocalResponse::Error(ErrorBody { ref reason, .. }) if reason == "deploy.bad_capsule"
        ));

        let resealed = handle_request(
            &daemon,
            LocalRequest::Recovery {
                action: "reseal".into(),
                provider: None,
                key_id: None,
                recovery_secret_hex: None,
            },
        );
        assert!(matches!(
            resealed,
            LocalResponse::Recovery { sealed: true, .. }
        ));
    }
}
