//! Connector contracts (DELIVERY D01–D02+).
//!
//! Object-store connectors hold Capsule bytes only. They never own desired
//! cluster state (RUNTIME.md §13 / DECISIONS D008).

#![forbid(unsafe_code)]

pub mod object;

pub use object::{
    final_capsule_key, quarantine_key, ByteRange, FakeObjectStore, ObjectEvidence, ObjectKey,
    ObjectStore, ObjectStoreError, ObjectStoreErrorKind, S3Config, S3ObjectStore, UploadId,
    UploadProgress,
};
