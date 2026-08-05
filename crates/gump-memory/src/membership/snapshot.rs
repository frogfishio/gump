//! In-memory snapshot transfer for joiners (PROTOCOL.md §14 / D001).

/// Offer from a live member: committed index + BLAKE3 of RAM snapshot bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotOffer {
    pub committed_index: u64,
    pub digest: [u8; 32],
    pub bytes: Vec<u8>,
}

impl SnapshotOffer {
    pub fn from_bytes(committed_index: u64, bytes: Vec<u8>) -> Self {
        let digest = *blake3::hash(&bytes).as_bytes();
        Self {
            committed_index,
            digest,
            bytes,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotTransferError {
    DigestMismatch {
        expected: [u8; 32],
        actual: [u8; 32],
    },
    IndexMismatch {
        expected: u64,
        actual: u64,
    },
}

impl std::fmt::Display for SnapshotTransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DigestMismatch { .. } => write!(f, "snapshot digest mismatch"),
            Self::IndexMismatch { expected, actual } => {
                write!(f, "snapshot index mismatch: expected {expected}, got {actual}")
            }
        }
    }
}

impl std::error::Error for SnapshotTransferError {}

/// Verifies digest and committed index before a joiner may leave Transferring.
pub trait SnapshotVerify {
    fn verify(
        &self,
        expected_index: u64,
        expected_digest: [u8; 32],
    ) -> Result<(), SnapshotTransferError>;
}

impl SnapshotVerify for SnapshotOffer {
    fn verify(
        &self,
        expected_index: u64,
        expected_digest: [u8; 32],
    ) -> Result<(), SnapshotTransferError> {
        let actual = *blake3::hash(&self.bytes).as_bytes();
        if actual != self.digest || actual != expected_digest {
            return Err(SnapshotTransferError::DigestMismatch {
                expected: expected_digest,
                actual,
            });
        }
        if self.committed_index != expected_index {
            return Err(SnapshotTransferError::IndexMismatch {
                expected: expected_index,
                actual: self.committed_index,
            });
        }
        Ok(())
    }
}
