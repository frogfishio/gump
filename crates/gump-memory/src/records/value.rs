//! Versioned record values (PROTOCOL.md §6–§7).

use crate::records::key::KeyPrefix;

/// Stored record body with revision and content digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordValue {
    pub revision: u64,
    pub digest: [u8; 32],
    pub payload: Vec<u8>,
    /// When set, bytes count toward the leased budget.
    pub leased: bool,
}

impl RecordValue {
    pub fn new(revision: u64, payload: Vec<u8>, leased: bool) -> Self {
        let digest = *blake3::hash(&payload).as_bytes();
        Self {
            revision,
            digest,
            payload,
            leased,
        }
    }

    pub fn byte_len(&self) -> u64 {
        self.payload.len() as u64
    }

    pub fn validate_size(&self, prefix: KeyPrefix) -> Result<(), ValueError> {
        let max = prefix.max_payload();
        if self.payload.len() > max {
            return Err(ValueError::PayloadTooLarge {
                len: self.payload.len(),
                max,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueError {
    PayloadTooLarge { len: usize, max: usize },
}

impl std::fmt::Display for ValueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PayloadTooLarge { len, max } => {
                write!(f, "payload length {len} exceeds prefix max {max}")
            }
        }
    }
}

impl std::error::Error for ValueError {}
