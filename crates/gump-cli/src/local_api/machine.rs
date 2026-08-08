//! Versioned machine-readable local API (C08 / GUMP-N006).

use serde::{Deserialize, Serialize};

pub const PROTOCOL_MAJOR: u32 = 1;
/// Minor 4: inventory / inspect / reintroduce full-loss recovery (GUMP-N016).
pub const PROTOCOL_MINOR: u32 = 4;

/// Client→daemon call with protocol negotiation, deadline, and cancel bit.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalCall {
    pub protocol_major: u32,
    pub protocol_minor: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancelled: bool,
    pub request: LocalRequest,
}

impl LocalCall {
    pub fn new(request: LocalRequest) -> Self {
        Self {
            protocol_major: PROTOCOL_MAJOR,
            protocol_minor: PROTOCOL_MINOR,
            deadline_ms: None,
            cancelled: false,
            request,
        }
    }

    pub fn with_deadline(mut self, deadline_ms: u64) -> Self {
        self.deadline_ms = Some(deadline_ms);
        self
    }
}

/// Stable machine-output envelope for CLI/daemon local replies.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MachineOutputV1 {
    pub schema: String,
    pub protocol_major: u32,
    pub protocol_minor: u32,
    pub body: LocalResponse,
}

impl MachineOutputV1 {
    pub fn wrap(body: LocalResponse) -> Self {
        Self {
            schema: "gump.local.machine.v1".into(),
            protocol_major: PROTOCOL_MAJOR,
            protocol_minor: PROTOCOL_MINOR,
            body,
        }
    }

    pub fn to_canonical_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum LocalRequest {
    Hello,
    Status,
    Explain {
        subject: String,
    },
    Observe {
        subject: String,
    },
    Deploy {
        operation_id: String,
        namespace: String,
        app: String,
        /// Lowercase hex BLAKE3-256 of the Capsule / desired payload.
        content_digest_hex: String,
        /// Optional lowercase hex of sealed Capsule bytes for upload→publish
        /// (GUMP-N010). Omit only when the final object already exists.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capsule_hex: Option<String>,
        /// Wait condition (`intent_accepted` default). See GUMP-N015 / D05.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wait: Option<String>,
    },
    Lifecycle {
        action: String,
        subject: String,
    },
    Recovery {
        action: String,
        /// `software` (default) or `fake-hsm` when `action` is `unseal`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key_id: Option<String>,
        /// Hex-encoded 32-byte recovery secret for software 1-of-1 unseal.
        /// Never echoed in responses/errors.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recovery_secret_hex: Option<String>,
    },
    ClusterAdmin {
        action: String,
    },
    /// Recent-window replay / live catch-up poll (memory-only; GUMP-N014).
    Telemetry {
        /// Exact topic, `prefix*`, or omit for all topics.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_events: Option<u32>,
    },
    /// List verified inert Capsules in the object store (GUMP-N016). Never activates.
    Inventory,
    /// Public Capsule metadata only — never protected values (GUMP-N016).
    Inspect {
        capsule_id: String,
    },
    /// Explicit full-loss recovery: fresh intent for a selected Capsule (GUMP-N016).
    Reintroduce {
        capsule_id: String,
        /// When true, verify + propose only; no K/V mutation (`reintroduce --plan`).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        plan: bool,
        /// Required for non-plan: `new_execution` or `resume`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        finite_mode: Option<String>,
        /// External checkpoint reference when `finite_mode` is `resume`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resume_from: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        operation_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        namespace: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app: Option<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LocalResponse {
    Hello {
        daemon: String,
        controller_epoch: u64,
    },
    Status(StatusBody),
    Explain {
        subject: String,
        reason_code: String,
        message: String,
        /// `committed_cluster_memory` or `observed` — never invents history.
        observation_source: String,
        /// True when the explain path discloses compaction / loss of detail.
        compaction_disclosed: bool,
        durability_note: String,
    },
    Observe {
        subject: String,
        state: String,
        detail: String,
    },
    Deploy {
        operation_id: String,
        phase: String,
        reason_code: String,
        safe_message: String,
        desired_generation: Option<u64>,
        content_digest_hex: String,
        durability_note: String,
        wait: DeployWaitBody,
        stages: Vec<DeployStageBody>,
        /// Interrupt/cancel/deadline never imply Capsule rollback (PROTOCOL §13).
        interrupted_implies_rollback: bool,
    },
    Lifecycle {
        action: String,
        subject: String,
        state: String,
        /// Always false; cancel/interrupt do not roll back published Capsules.
        interrupted_implies_rollback: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    Recovery {
        action: String,
        sealed: bool,
        requires_authority: bool,
        detail: String,
    },
    ClusterAdmin {
        action: String,
        memory_voters: u32,
        leader: Option<u64>,
        detail: String,
    },
    Telemetry {
        profile: String,
        memory_only: bool,
        pushed: u64,
        dropped_oldest: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<String>,
        caught_up: bool,
        identity_note: String,
        events: Vec<TelemetryEventBody>,
    },
    Inventory {
        desired_count: u64,
        note: String,
        capsules: Vec<InventoryEntryBody>,
    },
    Inspect {
        capsule_id: String,
        content_digest_hex: String,
        size_bytes: u64,
        object_key: String,
        live_referenced: bool,
        /// Public-only note; never includes protected config values.
        public_note: String,
    },
    Reintroduce {
        capsule_id: String,
        plan: bool,
        phase: String,
        reason_code: String,
        safe_message: String,
        content_digest_hex: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        finite_mode: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        desired_generation: Option<u64>,
        durability_note: String,
        /// Always false — reintroduce creates fresh intent; it does not restore history.
        restores_prior_desired: bool,
    },
    Error(ErrorBody),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InventoryEntryBody {
    pub capsule_id: String,
    pub content_digest_hex: String,
    pub size_bytes: u64,
    pub object_key: String,
    /// Whether this new/live cluster currently references the Capsule.
    pub live_referenced: bool,
    /// Always true for inventory listings — Capsules remain inert until reintroduce.
    pub inert: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TelemetryEventBody {
    Record {
        topic: String,
        stream_sequence: u64,
        utf8_hint: bool,
        bytes_hex: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    Gap {
        topic: String,
        from_sequence: u64,
        to_sequence: u64,
        reason: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StatusBody {
    pub cluster_id: String,
    pub incarnation: u64,
    pub controller_epoch: u64,
    pub controller_holder: Option<u64>,
    pub memory_voters: u32,
    pub durability_note: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeployWaitBody {
    pub condition: String,
    pub default_for_contract: String,
    pub matched_default: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeployStageBody {
    pub name: String,
    pub status: String,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    pub reason: String,
    pub safe_message: String,
}

pub fn sample_status() -> StatusBody {
    StatusBody {
        cluster_id: "00000000-0000-4000-8000-000000000001".into(),
        incarnation: 1,
        controller_epoch: 3,
        controller_holder: Some(1),
        memory_voters: 1,
        durability_note: "1 memory member; live intent has zero failure tolerance".into(),
    }
}

pub fn sample_hello_response() -> LocalResponse {
    LocalResponse::Hello {
        daemon: "gump-server".into(),
        controller_epoch: 3,
    }
}

pub fn sample_explain() -> LocalResponse {
    LocalResponse::Explain {
        subject: "unit/1".into(),
        reason_code: "placement.hard_filter".into(),
        message: "no eligible node matched hard requirements".into(),
        observation_source: "committed_cluster_memory".into(),
        compaction_disclosed: true,
        durability_note: "1 memory member; live intent has zero failure tolerance".into(),
    }
}

pub fn unauthorized_error() -> LocalResponse {
    LocalResponse::Error(ErrorBody {
        code: "UNAUTHORIZED".into(),
        reason: "peer.uid_denied".into(),
        safe_message: "local peer credentials rejected".into(),
    })
}

pub fn protocol_mismatch_error(client_major: u32, client_minor: u32) -> LocalResponse {
    LocalResponse::Error(ErrorBody {
        code: "PROTOCOL_MISMATCH".into(),
        reason: "protocol.version_unsupported".into(),
        safe_message: format!(
            "client {client_major}.{client_minor} incompatible with server {PROTOCOL_MAJOR}.{PROTOCOL_MINOR}"
        ),
    })
}

pub fn deadline_exceeded_error() -> LocalResponse {
    LocalResponse::Error(ErrorBody {
        code: "DEADLINE_EXCEEDED".into(),
        reason: "deadline.elapsed".into(),
        safe_message: "local call deadline elapsed before completion".into(),
    })
}

pub fn cancelled_error() -> LocalResponse {
    LocalResponse::Error(ErrorBody {
        code: "CANCELLED".into(),
        reason: "lifecycle.cancelled".into(),
        safe_message: "local call cancelled by client".into(),
    })
}

pub fn sample_observe() -> LocalResponse {
    LocalResponse::Observe {
        subject: "cluster".into(),
        state: "ready".into(),
        detail: "one memory member; zero failure tolerance".into(),
    }
}

pub fn sample_deploy() -> LocalResponse {
    use crate::local_api::receipt::{intent_accepted_stages, wait_body};
    LocalResponse::Deploy {
        operation_id: "00000000-0000-4000-8000-0000000000aa".into(),
        phase: "intent_accepted".into(),
        reason_code: "deploy.intent_accepted".into(),
        safe_message:
            "upload→publish→intent committed; placement/execution remains observed separately"
                .into(),
        desired_generation: Some(1),
        content_digest_hex: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .into(),
        durability_note: "1 memory member; live intent has zero failure tolerance".into(),
        wait: wait_body(None),
        stages: intent_accepted_stages(false),
        interrupted_implies_rollback: false,
    }
}

pub fn sample_lifecycle() -> LocalResponse {
    LocalResponse::Lifecycle {
        action: "interrupt".into(),
        subject: "attempt/1".into(),
        state: "acknowledged".into(),
        interrupted_implies_rollback: false,
        note: Some(
            "interrupt acknowledges loss of wait/observation; Capsule not rolled back".into(),
        ),
    }
}

pub fn sample_recovery() -> LocalResponse {
    LocalResponse::Recovery {
        action: "status".into(),
        sealed: true,
        requires_authority: true,
        detail: "cluster sealed; unseal authority required for new work".into(),
    }
}

pub fn sample_inventory() -> LocalResponse {
    LocalResponse::Inventory {
        desired_count: 0,
        note: "inert Capsules from object store; unreferenced ≠ obsolete or safe to delete".into(),
        capsules: vec![InventoryEntryBody {
            capsule_id: "00000000-0000-4000-8000-0000000000cc".into(),
            content_digest_hex:
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            size_bytes: 12,
            object_key: "clusters/00000000-0000-4000-8000-000000000001/capsules/00000000-0000-4000-8000-0000000000cc.capsule".into(),
            live_referenced: false,
            inert: true,
        }],
    }
}

pub fn sample_inspect() -> LocalResponse {
    LocalResponse::Inspect {
        capsule_id: "00000000-0000-4000-8000-0000000000cc".into(),
        content_digest_hex: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            .into(),
        size_bytes: 12,
        object_key: "clusters/00000000-0000-4000-8000-000000000001/capsules/00000000-0000-4000-8000-0000000000cc.capsule".into(),
        live_referenced: false,
        public_note: "public metadata only; protected values never printed".into(),
    }
}

pub fn sample_reintroduce() -> LocalResponse {
    LocalResponse::Reintroduce {
        capsule_id: "00000000-0000-4000-8000-0000000000cc".into(),
        plan: true,
        phase: "plan_ready".into(),
        reason_code: "reintroduce.plan".into(),
        safe_message: "verified inert Capsule; proposed fresh intent without mutating K/V".into(),
        content_digest_hex: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            .into(),
        finite_mode: Some("new_execution".into()),
        desired_generation: None,
        durability_note: "1 memory member; live intent has zero failure tolerance".into(),
        restores_prior_desired: false,
    }
}

pub fn sample_cluster_admin() -> LocalResponse {
    LocalResponse::ClusterAdmin {
        action: "members".into(),
        memory_voters: 1,
        leader: Some(1),
        detail: "one-voter cluster".into(),
    }
}

pub fn sample_telemetry() -> LocalResponse {
    LocalResponse::Telemetry {
        profile: "gump.ratatouille/1".into(),
        memory_only: true,
        pushed: 1,
        dropped_oldest: 0,
        filter: Some("app/stdout".into()),
        caught_up: true,
        identity_note:
            "canonical identity is placement-derived; producer hints are non-authoritative".into(),
        events: vec![TelemetryEventBody::Record {
            topic: "app/stdout".into(),
            stream_sequence: 0,
            utf8_hint: true,
            bytes_hex: "6869".into(),
            text: Some("hi".into()),
        }],
    }
}
