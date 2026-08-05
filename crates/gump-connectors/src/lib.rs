//! Connector contracts (DELIVERY D01–D03+).
//!
//! Object-store connectors hold Capsule bytes only. They never own desired
//! cluster state (RUNTIME.md §13 / DECISIONS D008). Streamed ingress (D03)
//! verifies sealed Capsules without unsealing.

#![forbid(unsafe_code)]

pub mod ingress;
pub mod object;

pub use ingress::{
    IngestStats, IngressError, IngressLimits, IngressReceipt, StreamedIngress,
    DEFAULT_MAX_CAPSULE_BYTES, DEFAULT_MAX_CHUNK_BYTES,
};
pub use object::{
    final_capsule_key, quarantine_key, ByteRange, FakeObjectStore, ObjectEvidence, ObjectKey,
    ObjectStore, ObjectStoreError, ObjectStoreErrorKind, S3Config, S3ObjectStore, UploadId,
    UploadProgress,
};
