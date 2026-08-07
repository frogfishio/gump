//! `ObjectStore` backed by an S3-compatible HTTP endpoint.

use std::collections::BTreeMap;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

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

#[derive(Debug)]
struct OpenUpload {
    expected_len: u64,
    written: u64,
    path: PathBuf,
    file: File,
    quarantine: ObjectKey,
}

/// S3-compatible connector: quarantine streams to a spill file then PUT;
/// promote uses streaming copy (no full-object Vec) (STL-03 / D008).
#[derive(Debug)]
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
        let path = std::env::temp_dir().join(format!(
            "gump-s3-q-{}-{}-{}.capsule",
            id.as_raw(),
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| {
                ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
            })?;
        self.uploads.insert(
            id,
            OpenUpload {
                expected_len,
                written: 0,
                path,
                file,
                quarantine,
            },
        );
        Ok(id)
    }

    fn write(
        &mut self,
        upload: UploadId,
        chunk: &[u8],
    ) -> Result<UploadProgress, ObjectStoreError> {
        let entry = self.uploads.get_mut(&upload).ok_or_else(|| {
            ObjectStoreError::new(ObjectStoreErrorKind::NotFound, "unknown upload")
        })?;
        let next = entry.written.saturating_add(chunk.len() as u64);
        if next > entry.expected_len {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::InvalidArgument,
                "write would exceed expected_len",
            ));
        }
        entry.file.write_all(chunk).map_err(|e| {
            ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
        })?;
        entry.written = next;
        Ok(UploadProgress {
            bytes_written: entry.written,
            expected_len: entry.expected_len,
        })
    }

    fn finish_quarantine(
        &mut self,
        upload: UploadId,
        digest: [u8; 32],
    ) -> Result<ObjectEvidence, ObjectStoreError> {
        let mut entry = self.uploads.remove(&upload).ok_or_else(|| {
            ObjectStoreError::new(ObjectStoreErrorKind::NotFound, "unknown upload")
        })?;
        if entry.written != entry.expected_len {
            let _ = std::fs::remove_file(&entry.path);
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::PreconditionFailed,
                format!(
                    "length {} != expected {}",
                    entry.written, entry.expected_len
                ),
            ));
        }
        entry.file.flush().map_err(|e| {
            ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
        })?;
        entry.file.seek(SeekFrom::Start(0)).map_err(|e| {
            ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
        })?;
        // Re-hash from spill to confirm caller digest without holding the object in RAM.
        let mut hasher = blake3::Hasher::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = entry.file.read(&mut buf).map_err(|e| {
                ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
            })?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let got = *hasher.finalize().as_bytes();
        if got != digest {
            let _ = std::fs::remove_file(&entry.path);
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::PreconditionFailed,
                "quarantine digest mismatch",
            ));
        }
        entry.file.seek(SeekFrom::Start(0)).map_err(|e| {
            ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
        })?;
        self.endpoint
            .put_from_reader(
                entry.quarantine.as_str(),
                &mut entry.file,
                entry.expected_len,
                digest,
                false,
            )
            .map_err(map_http)?;
        let _ = std::fs::remove_file(&entry.path);
        Ok(ObjectEvidence {
            key: entry.quarantine,
            length: entry.expected_len,
            digest,
        })
    }

    fn abort(&mut self, upload: UploadId) -> Result<(), ObjectStoreError> {
        let Some(entry) = self.uploads.remove(&upload) else {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::NotFound,
                "unknown upload",
            ));
        };
        let _ = std::fs::remove_file(&entry.path);
        Ok(())
    }

    fn publish_if_absent(
        &mut self,
        quarantine: &ObjectKey,
        final_key: &ObjectKey,
        digest: [u8; 32],
        len: u64,
    ) -> Result<ObjectEvidence, ObjectStoreError> {
        self.copy_if_absent(quarantine, final_key, digest, len)
    }

    fn head(&self, key: &ObjectKey) -> Result<ObjectEvidence, ObjectStoreError> {
        let meta = self.endpoint.head(key.as_str()).map_err(map_http)?;
        Ok(ObjectEvidence {
            key: key.clone(),
            length: meta.length,
            digest: meta.digest,
        })
    }

    fn get_reader(
        &self,
        key: &ObjectKey,
        range: Option<ByteRange>,
    ) -> Result<Box<dyn Read + '_>, ObjectStoreError> {
        let r = range.map(|b| (b.start, b.end));
        let reader = self
            .endpoint
            .get_reader(key.as_str(), r)
            .map_err(map_http)?;
        Ok(Box::new(reader))
    }

    fn copy_if_absent(
        &mut self,
        source: &ObjectKey,
        dest: &ObjectKey,
        digest: [u8; 32],
        len: u64,
    ) -> Result<ObjectEvidence, ObjectStoreError> {
        let q_meta = self.endpoint.head(source.as_str()).map_err(map_http)?;
        if q_meta.digest != digest || q_meta.length != len {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::PreconditionFailed,
                "quarantine evidence does not match publish args",
            ));
        }
        let mut reader = self
            .endpoint
            .get_reader(source.as_str(), None)
            .map_err(map_http)?;
        match self
            .endpoint
            .put_from_reader(dest.as_str(), &mut reader, len, digest, true)
        {
            Ok(()) => Ok(ObjectEvidence {
                key: dest.clone(),
                length: len,
                digest,
            }),
            Err(S3HttpError::Http { status: 412, .. }) => {
                let existing = self.endpoint.head(dest.as_str()).map_err(map_http)?;
                if existing.digest == digest && existing.length == len {
                    Ok(ObjectEvidence {
                        key: dest.clone(),
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

    fn delete(&mut self, key: &ObjectKey) -> Result<(), ObjectStoreError> {
        self.endpoint.delete(key.as_str()).map_err(map_http)
    }
}

fn map_http(e: S3HttpError) -> ObjectStoreError {
    match e {
        S3HttpError::Http { status: 404, .. } => {
            ObjectStoreError::new(ObjectStoreErrorKind::NotFound, e.to_string())
        }
        S3HttpError::Http {
            status: 409 | 412, ..
        } => ObjectStoreError::new(ObjectStoreErrorKind::Conflict, e.to_string()),
        other => ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, other.to_string()),
    }
}
