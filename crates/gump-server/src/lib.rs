//! Local CLI↔daemon Unix-domain API (C08 / DECISIONS D007).
//!
//! Peer credentials authenticate the connecting process. Valid peer identity is
//! necessary but never sufficient authorization for cluster mutations later.

pub mod framing;
pub mod machine;
pub mod peer;
pub mod serve;

pub use framing::{read_frame, write_frame, FrameError, MAX_FRAME_BYTES};
pub use machine::{
    sample_explain, sample_hello_response, sample_status, unauthorized_error, ErrorBody,
    LocalRequest, LocalResponse, MachineOutputV1, StatusBody, PROTOCOL_MAJOR, PROTOCOL_MINOR,
};
pub use peer::{PeerAllowlist, PeerAuthError, PeerCred};
pub use serve::{handle_request, serve_connection, LocalDaemon, ServeError};
