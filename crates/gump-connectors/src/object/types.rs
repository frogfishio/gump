//! Object-store types and trait.

use core::fmt;
use std::io::Read;

use gump_types::{CapsuleId, ClusterId};

/// Opaque upload handle for a quarantine session.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct UploadId(u64);

impl UploadId {
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn as_raw(self) -> u64 {
        self.0
    }
}

/// Exact object key under a cluster prefix.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ObjectKey(String);

impl ObjectKey {
    pub fn new(key: impl Into<String>) -> Result<Self, ObjectStoreError> {
        let key = key.into();
        if key.is_empty() || key.len() > 1024 || key.contains('\0') {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::InvalidKey,
                "object key empty, oversize, or contains NUL",
            ));
        }
        Ok(Self(key))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ObjectKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Optional byte range for `get`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ByteRange {
    pub start: u64,
    /// Exclusive end; `None` means through EOF.
    pub end: Option<u64>,
}

/// Immutable object evidence after head/publish/finish.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectEvidence {
    pub key: ObjectKey,
    pub length: u64,
    pub digest: [u8; 32],
}

/// Progress after writing a quarantine chunk.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadProgress {
    pub bytes_written: u64,
    pub expected_len: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ObjectStoreErrorKind {
    InvalidKey,
    InvalidArgument,
    NotFound,
    Conflict,
    PreconditionFailed,
    FaultInjected,
    Closed,
}

impl fmt::Display for ObjectStoreErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidKey => "invalid_key",
            Self::InvalidArgument => "invalid_argument",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::PreconditionFailed => "precondition_failed",
            Self::FaultInjected => "fault_injected",
            Self::Closed => "closed",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectStoreError {
    kind: ObjectStoreErrorKind,
    message: String,
}

impl ObjectStoreError {
    pub fn new(kind: ObjectStoreErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> ObjectStoreErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ObjectStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for ObjectStoreError {}

/// Object-store connector contract. Implementations store Capsule bytes only.
pub trait ObjectStore {
    fn begin_quarantine(
        &mut self,
        cluster: ClusterId,
        capsule: CapsuleId,
        expected_len: u64,
    ) -> Result<UploadId, ObjectStoreError>;

    fn write(&mut self, upload: UploadId, chunk: &[u8])
    -> Result<UploadProgress, ObjectStoreError>;

    fn finish_quarantine(
        &mut self,
        upload: UploadId,
        digest: [u8; 32],
    ) -> Result<ObjectEvidence, ObjectStoreError>;

    fn abort(&mut self, upload: UploadId) -> Result<(), ObjectStoreError>;

    /// Write-if-absent promotion. Matching digest+len is idempotent success.
    fn publish_if_absent(
        &mut self,
        quarantine: &ObjectKey,
        final_key: &ObjectKey,
        digest: [u8; 32],
        len: u64,
    ) -> Result<ObjectEvidence, ObjectStoreError>;

    fn head(&self, key: &ObjectKey) -> Result<ObjectEvidence, ObjectStoreError>;

    /// Streaming get (STL-03). Callers must not assume the body fits in RAM.
    fn get_reader(
        &self,
        key: &ObjectKey,
        range: Option<ByteRange>,
    ) -> Result<Box<dyn Read + '_>, ObjectStoreError>;

    /// Convenience: buffer a get. Prefer [`Self::get_reader`] for Capsule bodies.
    fn get(&self, key: &ObjectKey, range: Option<ByteRange>) -> Result<Vec<u8>, ObjectStoreError> {
        let mut reader = self.get_reader(key, range)?;
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).map_err(|e| {
            ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
        })?;
        Ok(buf)
    }

    /// Server-side / in-store copy for write-if-absent promote (no download).
    fn copy_if_absent(
        &mut self,
        source: &ObjectKey,
        dest: &ObjectKey,
        digest: [u8; 32],
        len: u64,
    ) -> Result<ObjectEvidence, ObjectStoreError>;

    fn delete(&mut self, key: &ObjectKey) -> Result<(), ObjectStoreError>;
}
