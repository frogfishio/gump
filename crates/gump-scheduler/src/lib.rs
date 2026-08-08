//! Feasibility, reservations, scoring, and admission (RUNTIME.md §1–2).
//!
//! Ownership: plans and reservations — not process control (DELIVERY.md).
//! Maps to GUMP-N011 / R01–R04.

#![forbid(unsafe_code)]

mod capability;
mod explain;
mod filter;
mod ledger;
mod place;
mod score;

pub use capability::{CapabilityReport, NodeResources, ProtectionLevel, WorkloadRequirements};
pub use explain::{ExplainReason, codes};
pub use filter::{hard_filter, with_headroom};
pub use ledger::{DEFAULT_MAX_NODES, DEFAULT_MAX_RESERVATIONS, Reservation, ResourceLedger};
pub use place::{NodeFeasibility, PlacementController, PlacementOutcome, PlacementPlan};
pub use score::score_residual_headroom;
