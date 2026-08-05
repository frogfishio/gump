//! `gump` CLI library — local `run` / sealed `test` (DELIVERY F07).

#![forbid(unsafe_code)]

mod error;
mod local;
mod sealed;

pub use error::{CliError, CliErrorKind};
pub use local::{local_parity_plan, run_local, LocalParityPlan, LocalRunOptions, LocalRunReport};
pub use sealed::{run_sealed_test, SealedTestOptions};
