//! `gump.driver/1` — stable driver trait and common lifecycle (DELIVERY F06).
//!
//! Authority: docs/v1/RUNTIME.md §4–§6, DECISIONS D009.

#![forbid(unsafe_code)]

mod abi;
mod common;
mod error;
mod native;
mod script;

pub use abi::{
    Admission, AttemptContext, Driver, DriverCapabilities, DriverKind, HostProbe, IoEndpoints,
    Observation, PreparedHandle, ReleaseRoot, ResourceGrant, RunningHandle, RuntimeSpec,
    SecretPlan, Signal, StartFence, DRIVER_ABI,
};
pub use error::{DriverError, DriverErrorKind};
pub use native::NativeDriver;
pub use script::ScriptDriver;
