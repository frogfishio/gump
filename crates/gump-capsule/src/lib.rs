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

pub use error::{CapsuleDialectError, CapsuleDialectErrorKind};
pub use header::{GumpCapsuleHeader, DIALECT, PAYLOAD_LAYOUT};
pub use segment::{
    SegmentDescriptor, SegmentTable, SegmentType, SEGMENT_COUNT, SEGMENT_DESC_LEN, TABLE_PREFIX_LEN,
};
pub use stream::{
    read_gump_capsule, write_gump_capsule, GumpCapsuleView, StreamingCapsuleReader,
    StreamingCapsuleWriter,
};
