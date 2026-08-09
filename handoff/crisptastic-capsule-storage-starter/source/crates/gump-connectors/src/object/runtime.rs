//! Runtime-selected Capsule store. Production uses S3; memory is an explicit
//! developer/test mode and is never selected implicitly by the server.

use std::io::Read;

use gump_types::{CapsuleId, ClusterId};

use super::{
    ByteRange, FakeObjectStore, ObjectEvidence, ObjectKey, ObjectStore, ObjectStoreError,
    S3ObjectStore, UploadId, UploadProgress,
};

#[derive(Debug)]
pub enum RuntimeObjectStore {
    Memory(FakeObjectStore),
    S3(S3ObjectStore),
}

impl RuntimeObjectStore {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Memory(_) => "memory-test",
            Self::S3(_) => "s3",
        }
    }
}

macro_rules! with_store {
    ($self:expr, $name:ident, $body:expr) => {
        match $self {
            RuntimeObjectStore::Memory($name) => $body,
            RuntimeObjectStore::S3($name) => $body,
        }
    };
}

impl ObjectStore for RuntimeObjectStore {
    fn begin_quarantine(
        &mut self,
        cluster: ClusterId,
        capsule: CapsuleId,
        expected_len: u64,
    ) -> Result<UploadId, ObjectStoreError> {
        with_store!(self, s, s.begin_quarantine(cluster, capsule, expected_len))
    }
    fn write(
        &mut self,
        upload: UploadId,
        chunk: &[u8],
    ) -> Result<UploadProgress, ObjectStoreError> {
        with_store!(self, s, s.write(upload, chunk))
    }
    fn finish_quarantine(
        &mut self,
        upload: UploadId,
        digest: [u8; 32],
    ) -> Result<ObjectEvidence, ObjectStoreError> {
        with_store!(self, s, s.finish_quarantine(upload, digest))
    }
    fn abort(&mut self, upload: UploadId) -> Result<(), ObjectStoreError> {
        with_store!(self, s, s.abort(upload))
    }
    fn publish_if_absent(
        &mut self,
        quarantine: &ObjectKey,
        final_key: &ObjectKey,
        digest: [u8; 32],
        len: u64,
    ) -> Result<ObjectEvidence, ObjectStoreError> {
        with_store!(
            self,
            s,
            s.publish_if_absent(quarantine, final_key, digest, len)
        )
    }
    fn head(&self, key: &ObjectKey) -> Result<ObjectEvidence, ObjectStoreError> {
        with_store!(self, s, s.head(key))
    }
    fn get_reader(
        &self,
        key: &ObjectKey,
        range: Option<ByteRange>,
    ) -> Result<Box<dyn Read + '_>, ObjectStoreError> {
        with_store!(self, s, s.get_reader(key, range))
    }
    fn copy_if_absent(
        &mut self,
        source: &ObjectKey,
        dest: &ObjectKey,
        digest: [u8; 32],
        len: u64,
    ) -> Result<ObjectEvidence, ObjectStoreError> {
        with_store!(self, s, s.copy_if_absent(source, dest, digest, len))
    }
    fn delete(&mut self, key: &ObjectKey) -> Result<(), ObjectStoreError> {
        with_store!(self, s, s.delete(key))
    }
    fn list_final_capsules(&self, limit: usize) -> Result<Vec<ObjectEvidence>, ObjectStoreError> {
        match self {
            Self::Memory(s) => ObjectStore::list_final_capsules(s, limit),
            Self::S3(s) => ObjectStore::list_final_capsules(s, limit),
        }
    }
}
