//! Directional sketch only. This file is intentionally not a compiled crate.

use std::io::Read;

pub enum Digest {
    Sha256([u8; 32]),
}

pub struct ObjectKey(String);

pub struct ObjectEvidence {
    pub key: ObjectKey,
    pub length: u64,
    pub digest: Digest,
}

pub trait ImmutableCapsuleStore {
    type Upload;
    type Error;

    fn begin_quarantine(
        &mut self,
        quarantine_key: ObjectKey,
        expected_length: u64,
    ) -> Result<Self::Upload, Self::Error>;

    fn write(&mut self, upload: &mut Self::Upload, bytes: &[u8])
        -> Result<(), Self::Error>;

    fn finish_quarantine(
        &mut self,
        upload: Self::Upload,
        expected_digest: Digest,
    ) -> Result<ObjectEvidence, Self::Error>;

    fn publish_if_absent(
        &mut self,
        quarantine: &ObjectKey,
        final_key: &ObjectKey,
        expected: &ObjectEvidence,
    ) -> Result<ObjectEvidence, Self::Error>;

    fn open<'a>(&'a self, key: &ObjectKey)
        -> Result<Box<dyn Read + 'a>, Self::Error>;

    fn head(&self, key: &ObjectKey) -> Result<ObjectEvidence, Self::Error>;
    fn delete(&mut self, key: &ObjectKey) -> Result<(), Self::Error>;
}

/// Crisptastic owns this policy, not the connector.
pub fn archived_capsule_key(sha256_hex: &str) -> Result<ObjectKey, &'static str> {
    if sha256_hex.len() != 64
        || !sha256_hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("capsule digest must be 64 lowercase hexadecimal characters");
    }
    Ok(ObjectKey(format!(
        "capsules/sha256/{}/{}.age",
        &sha256_hex[..2],
        sha256_hex
    )))
}
