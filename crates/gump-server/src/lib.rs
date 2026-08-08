//! Local CLI↔daemon Unix-domain API and product role composition (C08 / GUMP-N004–N006).
//!
//! Peer credentials authenticate the connecting process. Valid peer identity is
//! necessary but never sufficient authorization for cluster mutations later.
//!
//! The `gump` binary in this crate is the process entry point (docs/v1/README.md §5).

pub mod accept;
pub mod compose;
pub mod custody;
pub mod deploy_txn;
pub mod framing;
pub mod machine;
pub mod peer;
pub mod roles;
pub mod serve;

use gump_types::{HardenError, ProcessHardenReport, prepare_service_for_custody};

/// Early daemon hardening before accept/serve (STL-20 / SECURITY §8).
pub fn harden_daemon_startup() -> Result<ProcessHardenReport, HardenError> {
    prepare_service_for_custody()
}

pub use accept::{AcceptStats, CancelFlag, new_cancel_flag, run_accept_loop};
pub use compose::{InitOptions, ProductRuntime};
pub use custody::{ClusterCustody, CustodyError, CustodyStatus};
pub use framing::{FrameError, MAX_FRAME_BYTES, read_frame, write_frame};
pub use machine::{
    DeployStageBody, DeployWaitBody, ErrorBody, LocalCall, LocalRequest, LocalResponse,
    MachineOutputV1, PROTOCOL_MAJOR, PROTOCOL_MINOR, StatusBody, cancelled_error,
    deadline_exceeded_error, intent_accepted_stages, protocol_mismatch_error, sample_cluster_admin,
    sample_deploy, sample_explain, sample_hello_response, sample_lifecycle, sample_observe,
    sample_recovery, sample_status, sample_telemetry, unauthorized_error, wait_body,
};
pub use peer::{PeerAllowlist, PeerAuthError, PeerCred};
pub use roles::RoleSet;
pub use serve::{LocalDaemon, ServeError, handle_request, serve_connection};

#[cfg(test)]
mod stl20_tests {
    use super::harden_daemon_startup;

    #[test]
    fn daemon_startup_hardens_without_sealed_builder() {
        let report = harden_daemon_startup().expect("service Required policy");
        assert!(report.panic_hook_installed);
        #[cfg(unix)]
        assert!(report.core_dumps_disabled);
        assert!(report.to_string().contains("core_dumps="));
    }
}
