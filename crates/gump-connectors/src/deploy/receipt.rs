//! Truthful deploy receipts (CLI_LIFECYCLE.md §3 / CONFORMANCE).

use gump_types::{CapsuleId, WorkloadId};

use crate::deploy::types::ObjectLocator;
use crate::deploy::wait::WaitCondition;

/// Memory-loss / mutation guarantee surfaced on every receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurabilityGuarantee {
    pub memory_members: u32,
}

impl DurabilityGuarantee {
    pub fn describe(&self) -> String {
        if self.memory_members <= 1 {
            format!(
                "{} memory member; live intent has zero failure tolerance",
                self.memory_members.max(1)
            )
        } else {
            format!(
                "{} memory members; quorum required for live intent",
                self.memory_members
            )
        }
    }
}

/// Execution line on the receipt — never a vague “deployed”.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionStatus {
    /// Wait condition was only `accepted`.
    IntentAccepted,
    Converging {
        eligible: u32,
        total: u32,
    },
    ConditionMet {
        wait: WaitCondition,
        eligible: u32,
        total: u32,
    },
}

impl ExecutionStatus {
    pub fn describe(&self) -> String {
        match self {
            Self::IntentAccepted => "intent accepted — not waiting on units".into(),
            Self::Converging { eligible, total } => {
                format!("converging — {eligible}/{total} units eligible")
            }
            Self::ConditionMet {
                wait,
                eligible,
                total,
            } => format!("{} — {eligible}/{total} units eligible", wait.as_str()),
        }
    }
}

/// Stable deploy receipt fields (machine + human).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeployReceipt {
    pub application: String,
    pub capsule_id: CapsuleId,
    pub capsule_digest: [u8; 32],
    pub capsule_object: ObjectLocator,
    pub workload_id: WorkloadId,
    pub generation: u64,
    pub cluster_revision: u64,
    pub wait: WaitCondition,
    pub execution: ExecutionStatus,
    pub durability: DurabilityGuarantee,
    pub operation_id: [u8; 16],
}

/// Human-readable receipt (CLI_LIFECYCLE.md §3 example shape).
pub fn format_receipt_human(r: &DeployReceipt) -> String {
    let digest_hex = hex32(&r.capsule_digest);
    let short = &digest_hex[..8.min(digest_hex.len())];
    format!(
        "Application: {}\n\
Release:     {} / blake3:{}\n\
\n\
Capsule:     persisted in {}\n\
Intent:      accepted at cluster revision {}\n\
Execution:   {}\n\
Durability:  {}\n",
        r.application,
        short,
        digest_hex,
        r.capsule_object.uri,
        r.cluster_revision,
        r.execution.describe(),
        r.durability.describe(),
    )
}

fn hex32(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}
