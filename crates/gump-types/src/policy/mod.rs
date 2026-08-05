//! Policy engine and explicit action matrix (S01 / SECURITY.md §3).
//!
//! Roles are bundles only; enforcement always checks explicit actions.
//! Decisions are deny-by-default unless a grant or role membership allows.

mod action;
mod decision;
mod engine;
mod principal;
mod role;

pub use action::Action;
pub use decision::{Decision, DecisionEffect};
pub use engine::{PolicyEngine, PolicyError};
pub use principal::PrincipalId;
pub use role::Role;
