//! Disk-spilling fake object store for D01 overwrite/conflict/fault suite (STL-03b).
//!
//! Quarantine uploads and stored Capsule bodies live as spill files under a private
//! temp directory — not `Vec<u8>` — so large-object tests do not OOM the fake.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use gump_types::{CapsuleId, ClusterId};

use super::keys::{is_final_capsule_key, quarantine_key};
use super::types::{
    ByteRange, ObjectEvidence, ObjectKey, ObjectStore, ObjectStoreError, ObjectStoreErrorKind,
    UploadId, UploadProgress,
};

#[derive(Debug)]
struct StoredObject {
    path: PathBuf,
    length: u64,
    digest: [u8; 32],
}

#[derive(Debug)]
struct OpenUpload {
    expected_len: u64,
    written: u64,
    path: PathBuf,
    file: File,
    quarantine: ObjectKey,
}

/// Configurable fault knobs for the fake store.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FakeFaults {
    pub fail_next_write: bool,
    pub fail_next_publish: bool,
    pub fail_next_head: bool,
}

/// Object store for tests. Capsule bytes spill to disk (no desired-state map).
#[derive(Debug)]
pub struct FakeObjectStore {
    /// Declared first so it drops last (after uploads close FDs). STL-21.
    temp: tempfile::TempDir,
    objects: BTreeMap<ObjectKey, StoredObject>,
    uploads: BTreeMap<UploadId, OpenUpload>,
    next_upload: u64,
    pub faults: FakeFaults,
}

impl Default for FakeObjectStore {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeObjectStore {
    pub fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("gump-fake-store-")
            .tempdir()
            .expect("FakeObjectStore: create unique temp dir (STL-21)");
        Self {
            temp,
            objects: BTreeMap::new(),
            uploads: BTreeMap::new(),
            next_upload: 0,
            faults: FakeFaults::default(),
        }
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

    /// Inventory helper: final Capsule objects only (GUMP-N016). No activation.
    pub fn list_final_capsules(&self) -> Vec<ObjectEvidence> {
        self.objects
            .iter()
            .filter(|(k, _)| is_final_capsule_key(k))
            .map(|(k, o)| ObjectEvidence {
                key: k.clone(),
                length: o.length,
                digest: o.digest,
            })
            .collect()
    }

    fn spill_path(&self, tag: &str) -> PathBuf {
        self.temp.path().join(tag)
    }

    fn map_io(e: std::io::Error) -> ObjectStoreError {
        ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
    }

    fn open_rw(path: &Path) -> Result<File, ObjectStoreError> {
        OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(path)
            .map_err(Self::map_io)
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
        let path = self.spill_path(&format!("upload-{}.capsule", id.as_raw()));
        let file = Self::open_rw(&path)?;
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
        let next = entry.written.saturating_add(chunk.len() as u64);
        if next > entry.expected_len {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::InvalidArgument,
                "write would exceed expected_len",
            ));
        }
        entry.file.write_all(chunk).map_err(Self::map_io)?;
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
            let _ = fs::remove_file(&entry.path);
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::PreconditionFailed,
                format!(
                    "length {} != expected {}",
                    entry.written, entry.expected_len
                ),
            ));
        }
        entry.file.flush().map_err(Self::map_io)?;
        entry.file.seek(SeekFrom::Start(0)).map_err(Self::map_io)?;
        let mut hasher = blake3::Hasher::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = entry.file.read(&mut buf).map_err(Self::map_io)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let got = *hasher.finalize().as_bytes();
        if got != digest {
            let _ = fs::remove_file(&entry.path);
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::PreconditionFailed,
                "quarantine digest mismatch",
            ));
        }
        drop(entry.file);
        let final_path = self.spill_path(&format!(
            "obj-{}.capsule",
            entry.quarantine.as_str().replace('/', "_")
        ));
        fs::rename(&entry.path, &final_path).map_err(Self::map_io)?;
        let evidence = ObjectEvidence {
            key: entry.quarantine.clone(),
            length: entry.expected_len,
            digest,
        };
        self.objects.insert(
            entry.quarantine,
            StoredObject {
                path: final_path,
                length: entry.expected_len,
                digest,
            },
        );
        Ok(evidence)
    }

    fn abort(&mut self, upload: UploadId) -> Result<(), ObjectStoreError> {
        let Some(entry) = self.uploads.remove(&upload) else {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::NotFound,
                "unknown upload",
            ));
        };
        let _ = fs::remove_file(&entry.path);
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
            length: obj.length,
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
        let mut file = File::open(&obj.path).map_err(Self::map_io)?;
        let (start, end) = match range {
            None => (0, obj.length),
            Some(range) => {
                if range.start > obj.length {
                    return Err(ObjectStoreError::new(
                        ObjectStoreErrorKind::InvalidArgument,
                        "range start past EOF",
                    ));
                }
                let end = range.end.unwrap_or(obj.length);
                if end < range.start || end > obj.length {
                    return Err(ObjectStoreError::new(
                        ObjectStoreErrorKind::InvalidArgument,
                        "invalid byte range",
                    ));
                }
                (range.start, end)
            }
        };
        file.seek(SeekFrom::Start(start)).map_err(Self::map_io)?;
        Ok(Box::new(file.take(end.saturating_sub(start))))
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
        if q.digest != digest || q.length != len {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::PreconditionFailed,
                "quarantine evidence does not match publish args",
            ));
        }
        if let Some(existing) = self.objects.get(dest) {
            if existing.digest == digest && existing.length == len {
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
        let dest_path =
            self.spill_path(&format!("obj-{}.capsule", dest.as_str().replace('/', "_")));
        fs::copy(&q.path, &dest_path).map_err(Self::map_io)?;
        self.objects.insert(
            dest.clone(),
            StoredObject {
                path: dest_path,
                length: len,
                digest,
            },
        );
        Ok(ObjectEvidence {
            key: dest.clone(),
            length: len,
            digest,
        })
    }

    fn delete(&mut self, key: &ObjectKey) -> Result<(), ObjectStoreError> {
        let Some(obj) = self.objects.remove(key) else {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::NotFound,
                "object not found",
            ));
        };
        let _ = fs::remove_file(&obj.path);
        Ok(())
    }
}
