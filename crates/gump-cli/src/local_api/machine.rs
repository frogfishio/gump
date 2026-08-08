//! Versioned machine-readable local API (C08 / GUMP-N006).

use serde::{Deserialize, Serialize};

pub const PROTOCOL_MAJOR: u32 = 1;
/// Minor 1: request envelope + deploy/observe/lifecycle/recovery/cluster_admin ops.
pub const PROTOCOL_MINOR: u32 = 1;

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
    },
    Lifecycle {
        action: String,
        subject: String,
        state: String,
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
    Error(ErrorBody),
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
    LocalResponse::Deploy {
        operation_id: "00000000-0000-4000-8000-0000000000aa".into(),
        phase: "intent_accepted".into(),
        reason_code: "deploy.intent_accepted".into(),
        safe_message: "upload→publish→intent committed; placement/execution is N011/N012".into(),
        desired_generation: Some(1),
    }
}

pub fn sample_lifecycle() -> LocalResponse {
    LocalResponse::Lifecycle {
        action: "interrupt".into(),
        subject: "attempt/1".into(),
        state: "acknowledged".into(),
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

pub fn sample_cluster_admin() -> LocalResponse {
    LocalResponse::ClusterAdmin {
        action: "members".into(),
        memory_voters: 1,
        leader: Some(1),
        detail: "one-voter cluster".into(),
    }
}
