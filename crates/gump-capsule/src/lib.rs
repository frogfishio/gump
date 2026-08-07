//! `gump/deployment/1` Capsule dialect over Capsule v0001 (DELIVERY F04 / D003).
//!
//! Framing uses `capsule-lib` Encoding `C`. The payload is one CBOR byte string
//! whose contents are the GUMPDEP1 segment table + contiguous segments
//! (`docs/v1/FORMATS.md` §2–§3). Large Capsules are written/read through
//! streaming APIs that do not require buffering the full file to call
//! capsule-lib's byte-slice helpers.

#![forbid(unsafe_code)]

pub mod archive;
mod error;
mod header;
mod segment;
mod stream;
mod verify;

pub use error::{CapsuleDialectError, CapsuleDialectErrorKind};
pub use header::{DIALECT, GumpCapsuleHeader, PAYLOAD_LAYOUT};
pub use segment::{
    SEGMENT_COUNT, SEGMENT_DESC_LEN, SegmentDescriptor, SegmentTable, SegmentType, TABLE_BYTE_LEN,
    TABLE_PREFIX_LEN,
};
pub use stream::{
    DEFAULT_STREAM_CHUNK_BYTES, GumpCapsuleMeta, GumpCapsuleView, MAX_SIGNATURE_SEGMENT_BYTES,
    StreamingCapsuleReader, StreamingCapsuleWriter, read_gump_capsule, write_gump_capsule,
};
pub use verify::{release_signing_transcript, verify_release_signature};
