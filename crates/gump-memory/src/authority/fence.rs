//! Fence tokens carried on effect-creating commands (PROTOCOL.md §9).

/// Controller fence: epoch + opaque fence bytes + binding lease.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct FenceToken {
    pub epoch: u64,
    pub fence: [u8; 16],
    pub lease_id: u64,
}

impl FenceToken {
    pub fn new(epoch: u64, fence: [u8; 16], lease_id: u64) -> Self {
        Self {
            epoch,
            fence,
            lease_id,
        }
    }

    /// Deterministic fence material for tests / simulation (not a CSPRNG).
    pub fn derive_fence(epoch: u64, holder: u64, nonce: u64) -> [u8; 16] {
        let mut material = [0u8; 24];
        material[..8].copy_from_slice(&epoch.to_le_bytes());
        material[8..16].copy_from_slice(&holder.to_le_bytes());
        material[16..24].copy_from_slice(&nonce.to_le_bytes());
        let hash = blake3::hash(&material);
        let mut out = [0u8; 16];
        out.copy_from_slice(&hash.as_bytes()[..16]);
        out
    }
}

/// Why an effect under a presented fence was rejected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EffectReject {
    /// Presented epoch is below the current / accepted epoch.
    StaleEpoch {
        current: u64,
        presented: u64,
    },
    /// Same epoch but different fence bytes (protocol violation).
    FenceMismatch {
        epoch: u64,
    },
    /// Lease expired or unknown — fence unverifiable.
    ExpiredOrUnverifiable {
        lease_id: u64,
    },
    /// No controller authority has been acquired yet.
    NoAuthority,
}

impl std::fmt::Display for EffectReject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StaleEpoch {
                current,
                presented,
            } => write!(f, "stale controller epoch {presented} < current {current}"),
            Self::FenceMismatch { epoch } => {
                write!(f, "fence mismatch at epoch {epoch} (protocol violation)")
            }
            Self::ExpiredOrUnverifiable { lease_id } => {
                write!(f, "fence lease {lease_id} expired or unverifiable")
            }
            Self::NoAuthority => write!(f, "no controller authority"),
        }
    }
}

impl std::error::Error for EffectReject {}
