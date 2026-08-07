//! Deploy workflow: receipt, wait, retry, orphan handling (D05).
//!
//! Authority: CLI_LIFECYCLE.md §3 / deploy waits, PROTOCOL.md §13–§15,
//! CONFORMANCE Deploy receipt, DECISIONS D014.

mod idempotency;
mod receipt;
mod types;
mod wait;
mod workflow;

pub use idempotency::{IdempotencyCache, IdempotencyError, IdempotencyRecord};
pub use receipt::{DeployReceipt, DurabilityGuarantee, ExecutionStatus, format_receipt_human};
pub use types::{
    ConvergenceSnapshot, DeployFailure, DeployOutcome, DeployPhase, ObjectLocator, OrphanCapsule,
    WorkloadContract,
};
pub use wait::{WaitCondition, default_wait_condition};
pub use workflow::{DeployBackend, DeployRequest, DeployWorkflow};
