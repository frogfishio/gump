//! `gump` CLI library — local `run` / sealed `test` (DELIVERY F07 / X01).

#![forbid(unsafe_code)]

mod error;
mod local;
mod sealed;

pub use error::{CliError, CliErrorKind};
pub use local::{LocalParityPlan, LocalRunOptions, LocalRunReport, local_parity_plan, run_local};
pub use sealed::{
    BuiltSealedCapsule, SealedTestOptions, build_sealed_capsule, run_sealed_test,
    run_verified_sealed, verify_sealed_capsule,
};
