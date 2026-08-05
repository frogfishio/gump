//! Shared foundation types for Gump (DELIVERY W02 / DECISIONS D001–D002).
//!
//! Product crates depend inward on this layer. It must not depend on transport,
//! memory, CLI, or vendor protocol SDKs.
//!
//! The [`sim`] module is the W05 deterministic simulation harness. The [`policy`]
//! module is the S01 deny-by-default action matrix (SECURITY.md §3).

#![forbid(unsafe_code)]

mod bounded;
mod cancel;
mod clock;
mod error;
mod id;
pub mod policy;
mod secret;
pub mod sim;

pub use bounded::{BoundedString, Label, LabelError};
pub use cancel::{CancelToken, CancellationGuard, Cancelled};
pub use clock::{Clock, DurationMillis, InstantMillis, ManualClock, SystemClock};
pub use error::{ReasonCode, SafeError};
pub use id::{
    AttemptId, CapsuleId, ClusterId, ExecutionId, GumpId, IdError, IncarnationId, LeaseId,
    MessageId, NodeId, OperationId, PlacementGroupId, UnitId, WorkloadId,
};
pub use policy::{
    Action, Decision, DecisionEffect, PolicyEngine, PolicyError, PrincipalId, Role,
};
pub use secret::Secret;
