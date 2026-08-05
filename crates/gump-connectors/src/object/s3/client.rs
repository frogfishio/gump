//! `ObjectStore` backed by an S3-compatible HTTP endpoint.

use std::collections::BTreeMap;

use gump_types::{CapsuleId, ClusterId};

use crate::object::keys::quarantine_key;
use crate::object::s3::http::{S3Endpoint, S3HttpError};
use crate::object::types::{
    ByteRange, ObjectEvidence, ObjectKey, ObjectStore, ObjectStoreError, ObjectStoreErrorKind,
    UploadId, UploadProgress,
};

#[derive(Clone, Debug)]
pub struct S3Config {
    pub host: String,
    pub port: u16,
    pub bucket: String,
}

impl S3Config {
    pub fn new(host: impl Into<String>, port: u16, bucket: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port,
            bucket: bucket.into(),
        }
    }
}

#[derive(Clone, Debug)]
struct OpenUpload {
    expected_len: u64,
    buffer: Vec<u8>,
    quarantine: ObjectKey,
}

/// S3-compatible connector: quarantine via ordinary PUT; final publish uses
/// `If-None-Match: *` (write-if-absent). Matching digest+len is idempotent.
#[derive(Clone, Debug)]
pub struct S3ObjectStore {
    endpoint: S3Endpoint,
    uploads: BTreeMap<UploadId, OpenUpload>,
    next_upload: u64,
}

impl S3ObjectStore {
    pub fn new(config: S3Config) -> Self {
        Self {
            endpoint: S3Endpoint {
                host: config.host,
                port: config.port,
                bucket: config.bucket,
            },
            uploads: BTreeMap::new(),
            next_upload: 0,
        }
    }
}

impl ObjectStore for S3ObjectStore {
    fn begin_quarantine(
        &mut self,
        cluster: ClusterId,
        capsule: CapsuleId,
        expected_len: u64,
    ) -> Result<UploadId, ObjectStoreError> {
        if expected_len == 0 {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::InvalidArgument,
                "expected_len must be non-zero",
            ));
        }
        self.next_upload = self.next_upload.saturating_add(1);
        let id = UploadId::from_raw(self.next_upload);
        let quarantine = quarantine_key(cluster, capsule, id.as_raw())?;
        self.uploads.insert(
            id,
            OpenUpload {
                expected_len,
                buffer: Vec::with_capacity(expected_len.min(1024 * 1024) as usize),
                quarantine,
            },
        );
        Ok(id)
    }

    fn write(&mut self, upload: UploadId, chunk: &[u8]) -> Result<UploadProgress, ObjectStoreError> {
        let entry = self.uploads.get_mut(&upload).ok_or_else(|| {
            ObjectStoreError::new(ObjectStoreErrorKind::NotFound, "unknown upload")
        })?;
        let next = entry.buffer.len() as u64 + chunk.len() as u64;
        if next > entry.expected_len {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::InvalidArgument,
                "write would exceed expected_len",
            ));
        }
        entry.buffer.extend_from_slice(chunk);
        Ok(UploadProgress {
            bytes_written: entry.buffer.len() as u64,
            expected_len: entry.expected_len,
        })
    }

    fn finish_quarantine(
        &mut self,
        upload: UploadId,
        digest: [u8; 32],
    ) -> Result<ObjectEvidence, ObjectStoreError> {
        let entry = self.uploads.remove(&upload).ok_or_else(|| {
            ObjectStoreError::new(ObjectStoreErrorKind::NotFound, "unknown upload")
        })?;
        if entry.buffer.len() as u64 != entry.expected_len {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::PreconditionFailed,
                format!(
                    "length {} != expected {}",
                    entry.buffer.len(),
                    entry.expected_len
                ),
            ));
        }
        let got = *blake3::hash(&entry.buffer).as_bytes();
        if got != digest {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::PreconditionFailed,
                "quarantine digest mismatch",
            ));
        }
        self.endpoint
            .put(entry.quarantine.as_str(), &entry.buffer, digest, false)
            .map_err(map_http)?;
        Ok(ObjectEvidence {
            key: entry.quarantine,
            length: entry.expected_len,
            digest,
        })
    }

    fn abort(&mut self, upload: UploadId) -> Result<(), ObjectStoreError> {
        if self.uploads.remove(&upload).is_none() {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::NotFound,
                "unknown upload",
            ));
        }
        Ok(())
    }

    fn publish_if_absent(
        &mut self,
        quarantine: &ObjectKey,
        final_key: &ObjectKey,
        digest: [u8; 32],
        len: u64,
    ) -> Result<ObjectEvidence, ObjectStoreError> {
        let q_meta = self.endpoint.head(quarantine.as_str()).map_err(map_http)?;
        if q_meta.digest != digest || q_meta.length != len {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::PreconditionFailed,
                "quarantine evidence does not match publish args",
            ));
        }
        let bytes = self.endpoint.get(quarantine.as_str(), None).map_err(map_http)?;
        match self
            .endpoint
            .put(final_key.as_str(), &bytes, digest, true)
        {
            Ok(()) => Ok(ObjectEvidence {
                key: final_key.clone(),
                length: len,
                digest,
            }),
            Err(S3HttpError::Http { status: 412, .. }) => {
                // Pre-existing: accept only exact digest+length match (D008).
                let existing = self.endpoint.head(final_key.as_str()).map_err(map_http)?;
                if existing.digest == digest && existing.length == len {
                    Ok(ObjectEvidence {
                        key: final_key.clone(),
                        length: len,
                        digest,
                    })
                } else {
                    Err(ObjectStoreError::new(
                        ObjectStoreErrorKind::Conflict,
                        "final key occupied by different object",
                    ))
                }
            }
            Err(e) => Err(map_http(e)),
        }
    }

    fn head(&self, key: &ObjectKey) -> Result<ObjectEvidence, ObjectStoreError> {
        let meta = self.endpoint.head(key.as_str()).map_err(map_http)?;
        Ok(ObjectEvidence {
            key: key.clone(),
            length: meta.length,
            digest: meta.digest,
        })
    }

    fn get(&self, key: &ObjectKey, range: Option<ByteRange>) -> Result<Vec<u8>, ObjectStoreError> {
        let r = range.map(|b| (b.start, b.end));
        self.endpoint.get(key.as_str(), r).map_err(map_http)
    }

    fn delete(&mut self, key: &ObjectKey) -> Result<(), ObjectStoreError> {
        self.endpoint.delete(key.as_str()).map_err(map_http)
    }
}

fn map_http(e: S3HttpError) -> ObjectStoreError {
    match e {
        S3HttpError::Http { status: 404, .. } => {
            ObjectStoreError::new(ObjectStoreErrorKind::NotFound, e.to_string())
        }
        S3HttpError::Http { status: 409 | 412, .. } => {
            ObjectStoreError::new(ObjectStoreErrorKind::Conflict, e.to_string())
        }
        other => ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, other.to_string()),
    }
}
