//! Protobuf messages, control-frame limits, golden vectors, and session
//! negotiation (W03 / C01).
//!
//! Authority: docs/v1/PROTOCOL.md §1–§3, docs/v1/FORMATS.md §4, DECISIONS D001.

#![forbid(unsafe_code)]

pub mod frame;
pub mod goldens;
pub mod negotiate;

/// Generated `gump.v1` messages from `proto/gump/v1/*.proto`.
pub mod pb {
    include!(concat!(env!("OUT_DIR"), "/gump.v1.rs"));
}

pub use frame::{
    FrameError, FrameKind, MAX_CONTROL_FRAME, MAX_ERROR_FRAME, MAX_HELLO_FRAME,
    decode_frame_prefix, encode_frame,
};
pub use negotiate::{
    local_hello, negotiate_hello, validate_envelope, NegotiateError, NegotiatedSession,
    ProtocolSupport, PROTOCOL_MAJOR, PROTOCOL_MINOR,
};
