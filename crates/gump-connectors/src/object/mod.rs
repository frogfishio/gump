//! Object-store connector ABI (RUNTIME.md §13, DECISIONS D008).

mod fake;
mod keys;
mod runtime;
mod s3;
mod types;

pub use fake::FakeObjectStore;
pub use keys::{final_capsule_key, is_final_capsule_key, parse_final_capsule_key, quarantine_key};
pub use runtime::RuntimeObjectStore;
pub use s3::{META_BLAKE3, S3Config, S3ObjectStore};
pub use types::{
    ByteRange, ObjectEvidence, ObjectKey, ObjectStore, ObjectStoreError, ObjectStoreErrorKind,
    UploadId, UploadProgress,
};
