//! `gump` CLI library — local `run` / sealed `test` (DELIVERY F07 / X01)
//! and cluster-backed local API client (C08 / GUMP-N006).

#![forbid(unsafe_code)]

mod cmd;
mod error;
mod local;
mod local_api;
mod packaging;
mod sealed;

pub use cmd::{dispatch_cli, print_help, try_dispatch_cli};
pub use error::{CliError, CliErrorKind};
pub use local::{LocalParityPlan, LocalRunOptions, LocalRunReport, local_parity_plan, run_local};
pub use local_api::{
    ErrorBody, FrameError, LocalCall, LocalClient, LocalClientError, LocalRequest, LocalResponse,
    MAX_FRAME_BYTES, MachineOutputV1, PROTOCOL_MAJOR, PROTOCOL_MINOR, StatusBody, cancelled_error,
    deadline_exceeded_error, protocol_mismatch_error, read_frame, sample_cluster_admin,
    sample_deploy, sample_explain, sample_hello_response, sample_lifecycle, sample_observe,
    sample_recovery, sample_status, unauthorized_error, write_frame,
};
pub use sealed::{
    BuiltSealedCapsule, SealedTestOptions, build_sealed_capsule, run_sealed_test,
    run_verified_sealed, verify_sealed_capsule,
};
