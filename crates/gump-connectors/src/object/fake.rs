//! In-memory fake object store for D01 overwrite/conflict/fault suite.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use gump_types::{CapsuleId, ClusterId};

use super::keys::quarantine_key;
use super::types::{
    ByteRange, ObjectEvidence, ObjectKey, ObjectStore, ObjectStoreError, ObjectStoreErrorKind,
    UploadId, UploadProgress,
};

#[derive(Clone, Debug)]
struct StoredObject {
    bytes: Vec<u8>,
    digest: [u8; 32],
}

#[derive(Clone, Debug)]
struct OpenUpload {
    cluster: ClusterId,
    capsule: CapsuleId,
    expected_len: u64,
    buffer: Vec<u8>,
    quarantine: ObjectKey,
}

/// Configurable fault knobs for the fake store.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FakeFaults {
    pub fail_next_write: bool,
    pub fail_next_publish: bool,
    pub fail_next_head: bool,
}

/// In-memory object store. Holds Capsule bytes only — no desired-state map.
#[derive(Clone, Debug, Default)]
pub struct FakeObjectStore {
    objects: BTreeMap<ObjectKey, StoredObject>,
    uploads: BTreeMap<UploadId, OpenUpload>,
    next_upload: u64,
    pub faults: FakeFaults,
}

impl FakeObjectStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    pub fn open_upload_count(&self) -> usize {
        self.uploads.len()
    }

    /// Test helper: assert the store has no workload/desired-state keys.
    pub fn keys(&self) -> Vec<ObjectKey> {
        self.objects.keys().cloned().collect()
    }
}

impl ObjectStore for FakeObjectStore {
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
                cluster,
                capsule,
                expected_len,
                buffer: Vec::with_capacity(expected_len.min(1024 * 1024) as usize),
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
        if self.faults.fail_next_write {
            self.faults.fail_next_write = false;
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::FaultInjected,
                "injected write fault",
            ));
        }
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
        let got = blake3::hash(&entry.buffer);
        if got.as_bytes() != &digest {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::PreconditionFailed,
                "quarantine digest mismatch",
            ));
        }
        let evidence = ObjectEvidence {
            key: entry.quarantine.clone(),
            length: entry.expected_len,
            digest,
        };
        self.objects.insert(
            entry.quarantine,
            StoredObject {
                bytes: entry.buffer,
                digest,
            },
        );
        let _ = (entry.cluster, entry.capsule);
        Ok(evidence)
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
        self.copy_if_absent(quarantine, final_key, digest, len)
    }

    fn head(&self, key: &ObjectKey) -> Result<ObjectEvidence, ObjectStoreError> {
        // Sticky until the test clears `faults.fail_next_head` (head is &self).
        if self.faults.fail_next_head {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::FaultInjected,
                "injected head fault",
            ));
        }
        let obj = self.objects.get(key).ok_or_else(|| {
            ObjectStoreError::new(ObjectStoreErrorKind::NotFound, "object not found")
        })?;
        Ok(ObjectEvidence {
            key: key.clone(),
            length: obj.bytes.len() as u64,
            digest: obj.digest,
        })
    }

    fn get_reader(
        &self,
        key: &ObjectKey,
        range: Option<ByteRange>,
    ) -> Result<Box<dyn Read + '_>, ObjectStoreError> {
        let obj = self.objects.get(key).ok_or_else(|| {
            ObjectStoreError::new(ObjectStoreErrorKind::NotFound, "object not found")
        })?;
        let bytes = match range {
            None => obj.bytes.clone(),
            Some(range) => {
                let start = range.start as usize;
                if start > obj.bytes.len() {
                    return Err(ObjectStoreError::new(
                        ObjectStoreErrorKind::InvalidArgument,
                        "range start past EOF",
                    ));
                }
                let end = match range.end {
                    Some(e) => e as usize,
                    None => obj.bytes.len(),
                };
                if end < start || end > obj.bytes.len() {
                    return Err(ObjectStoreError::new(
                        ObjectStoreErrorKind::InvalidArgument,
                        "invalid byte range",
                    ));
                }
                obj.bytes[start..end].to_vec()
            }
        };
        Ok(Box::new(Cursor::new(bytes)))
    }

    fn copy_if_absent(
        &mut self,
        source: &ObjectKey,
        dest: &ObjectKey,
        digest: [u8; 32],
        len: u64,
    ) -> Result<ObjectEvidence, ObjectStoreError> {
        if self.faults.fail_next_publish {
            self.faults.fail_next_publish = false;
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::FaultInjected,
                "injected publish fault",
            ));
        }
        let q = self.objects.get(source).ok_or_else(|| {
            ObjectStoreError::new(ObjectStoreErrorKind::NotFound, "quarantine object missing")
        })?;
        if q.digest != digest || q.bytes.len() as u64 != len {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::PreconditionFailed,
                "quarantine evidence does not match publish args",
            ));
        }
        if let Some(existing) = self.objects.get(dest) {
            if existing.digest == digest && existing.bytes.len() as u64 == len {
                return Ok(ObjectEvidence {
                    key: dest.clone(),
                    length: len,
                    digest,
                });
            }
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::Conflict,
                "final key occupied by different object",
            ));
        }
        let stored = StoredObject {
            bytes: q.bytes.clone(),
            digest,
        };
        self.objects.insert(dest.clone(), stored);
        Ok(ObjectEvidence {
            key: dest.clone(),
            length: len,
            digest,
        })
    }

    fn delete(&mut self, key: &ObjectKey) -> Result<(), ObjectStoreError> {
        if self.objects.remove(key).is_none() {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::NotFound,
                "object not found",
            ));
        }
        Ok(())
    }
}
