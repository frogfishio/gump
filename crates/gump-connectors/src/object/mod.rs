//! Object-store connector ABI (RUNTIME.md §13, DECISIONS D008).

mod fake;
mod keys;
mod types;

pub use fake::FakeObjectStore;
pub use keys::{final_capsule_key, quarantine_key};
pub use types::{
    ByteRange, ObjectEvidence, ObjectKey, ObjectStore, ObjectStoreError, ObjectStoreErrorKind,
    UploadId, UploadProgress,
};
