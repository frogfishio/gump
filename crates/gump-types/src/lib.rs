//! Shared foundation types for Gump (DELIVERY W02 / DECISIONS D001–D002).
//!
//! Product crates depend inward on this layer. It must not depend on transport,
//! memory, CLI, or vendor protocol SDKs.

#![forbid(unsafe_code)]

mod bounded;
mod cancel;
mod clock;
mod error;
mod id;
mod secret;

pub use bounded::{BoundedString, Label, LabelError};
pub use cancel::{CancelToken, CancellationGuard, Cancelled};
pub use clock::{Clock, DurationMillis, InstantMillis, ManualClock, SystemClock};
pub use error::{ReasonCode, SafeError};
pub use id::{
    AttemptId, CapsuleId, ClusterId, ExecutionId, GumpId, IdError, IncarnationId, LeaseId,
    MessageId, NodeId, OperationId, PlacementGroupId, UnitId, WorkloadId,
};
pub use secret::Secret;
