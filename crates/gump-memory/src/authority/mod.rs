//! Controller election and fencing (C07 / PROTOCOL.md §9).

mod agent;
mod controller;
mod fence;

pub use agent::{AgentFenceError, AgentFenceMemory};
pub use controller::{ControllerAuthority, ControllerError, EffectCommand};
pub use fence::{EffectReject, FenceToken};
