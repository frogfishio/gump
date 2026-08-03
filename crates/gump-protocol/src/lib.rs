//! Protobuf messages, control-frame limits, and golden vectors (W03).
//!
//! Authority: docs/v1/PROTOCOL.md §2, docs/v1/FORMATS.md §4, DECISIONS D001.

#![forbid(unsafe_code)]

pub mod frame;
pub mod goldens;

/// Generated `gump.v1` messages from `proto/gump/v1/*.proto`.
pub mod pb {
    include!(concat!(env!("OUT_DIR"), "/gump.v1.rs"));
}

pub use frame::{
    decode_frame_prefix, encode_frame, FrameError, FrameKind, MAX_CONTROL_FRAME, MAX_ERROR_FRAME,
    MAX_HELLO_FRAME,
};
