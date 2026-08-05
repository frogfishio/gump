//! Deploy workflow types (failure phases, orphans, contracts).

use gump_types::{CapsuleId, WorkloadId};

use crate::deploy::wait::WaitCondition;
use crate::object::ObjectKey;

/// Declared workload shape used to pick default waits (not a full manifest).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkloadContract {
    pub app_name: String,
    pub lifecycle_finite: bool,
    pub declares_readiness: bool,
    pub requires_publication: bool,
    pub is_gang: bool,
    pub units: u32,
}

/// Where a Capsule object lives after immutable publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectLocator {
    pub key: ObjectKey,
    pub uri: String,
}

/// Phase at which a deploy attempt failed (CONFORMANCE failure taxonomy).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeployPhase {
    LocalValidation,
    Authz,
    CapsulePersist,
    ImmutablePublish,
    IntentAccept,
    Scheduling,
    Execution,
    WaitObservation,
}

impl DeployPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalValidation => "local_validation",
            Self::Authz => "authorization",
            Self::CapsulePersist => "capsule_persist",
            Self::ImmutablePublish => "immutable_publish",
            Self::IntentAccept => "intent_accept",
            Self::Scheduling => "scheduling",
            Self::Execution => "execution",
            Self::WaitObservation => "wait_observation",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeployFailure {
    pub phase: DeployPhase,
    pub reason: String,
    /// Present when Capsule bytes were published but intent was not accepted.
    pub orphan: Option<OrphanCapsule>,
}

/// Inert Capsule left after publish-without-intent (PROTOCOL.md §13).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrphanCapsule {
    pub capsule_id: CapsuleId,
    pub capsule_digest: [u8; 32],
    pub object: ObjectLocator,
    pub operation_id: [u8; 16],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeployOutcome {
    Success(crate::deploy::receipt::DeployReceipt),
    Failed(DeployFailure),
    /// Same operation ID, different request digest (PROTOCOL.md §15).
    Conflict {
        operation_id: [u8; 16],
    },
}

/// Observed convergence relative to a wait condition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConvergenceSnapshot {
    pub wait: WaitCondition,
    pub satisfied: bool,
    pub units_eligible: u32,
    pub units_total: u32,
    pub workload_id: WorkloadId,
    pub generation: u64,
    pub cluster_revision: u64,
    pub memory_members: u32,
}
