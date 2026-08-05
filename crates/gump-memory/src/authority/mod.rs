//! Controller election and fencing (C07 / PROTOCOL.md §9).

mod agent;
mod controller;
mod fence;

pub use agent::{AgentFenceMemory, AgentFenceError};
pub use controller::{ControllerAuthority, ControllerError, EffectCommand};
pub use fence::{EffectReject, FenceToken};
