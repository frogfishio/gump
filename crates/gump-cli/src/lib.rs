//! `gump` CLI library — local `run` / sealed `test` (DELIVERY F07 / X01).

#![forbid(unsafe_code)]

mod error;
mod local;
mod sealed;

pub use error::{CliError, CliErrorKind};
pub use local::{local_parity_plan, run_local, LocalParityPlan, LocalRunOptions, LocalRunReport};
pub use sealed::{
    build_sealed_capsule, run_sealed_test, run_verified_sealed, verify_sealed_capsule,
    BuiltSealedCapsule, SealedTestOptions,
};
