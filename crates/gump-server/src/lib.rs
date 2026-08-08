//! Local CLI↔daemon Unix-domain API (C08 / DECISIONS D007).
//!
//! Peer credentials authenticate the connecting process. Valid peer identity is
//! necessary but never sufficient authorization for cluster mutations later.

pub mod framing;
pub mod machine;
pub mod peer;
pub mod serve;

use gump_types::{HardenError, ProcessHardenReport, prepare_service_for_custody};

/// Early daemon hardening before accept/serve (STL-20 / SECURITY §8).
/// Surfaces [`ProcessHardenReport`]; fails closed when service policy requires.
pub fn harden_daemon_startup() -> Result<ProcessHardenReport, HardenError> {
    prepare_service_for_custody()
}

pub use framing::{FrameError, MAX_FRAME_BYTES, read_frame, write_frame};
pub use machine::{
    ErrorBody, LocalRequest, LocalResponse, MachineOutputV1, PROTOCOL_MAJOR, PROTOCOL_MINOR,
    StatusBody, sample_explain, sample_hello_response, sample_status, unauthorized_error,
};
pub use peer::{PeerAllowlist, PeerAuthError, PeerCred};
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
