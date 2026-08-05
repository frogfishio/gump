//! Versioned machine-readable local API messages (CLI machine output).

use serde::{Deserialize, Serialize};

pub const PROTOCOL_MAJOR: u32 = 1;
pub const PROTOCOL_MINOR: u32 = 0;

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
        // serde_json preserves struct field order from derive; stable enough for goldens.
        serde_json::to_string_pretty(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum LocalRequest {
    Hello,
    Status,
    Explain { subject: String },
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

/// Deterministic status used for machine-output goldens.
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
