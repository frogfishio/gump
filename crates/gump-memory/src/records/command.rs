//! State-machine commands (PROTOCOL.md §6, §8).

use crate::records::key::RecordKey;
use crate::records::lease::LeasePurpose;

/// Compare precondition for Put/Delete/Txn.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Expected {
    /// Key must not exist.
    Absent,
    /// Key must exist at exactly this revision.
    ExactRevision(u64),
    /// Key must exist with this BLAKE3 digest.
    ExactDigest([u8; 32]),
    /// No precondition.
    Any,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MutateOp {
    Put {
        key: RecordKey,
        expected: Expected,
        payload: Vec<u8>,
        leased: bool,
    },
    Delete {
        key: RecordKey,
        expected: Expected,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Comparison {
    pub key: RecordKey,
    pub expected: Expected,
}

/// Atomic multi-key transaction at one new revision.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Txn {
    pub comparisons: Vec<Comparison>,
    pub success_ops: Vec<MutateOp>,
    pub failure_ops: Vec<MutateOp>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Command {
    Put {
        key: RecordKey,
        expected: Expected,
        payload: Vec<u8>,
        leased: bool,
    },
    Delete {
        key: RecordKey,
        expected: Expected,
    },
    Txn(Txn),
    /// Raise watch compaction floor through `through` (inclusive).
    Compact {
        through: u64,
    },
    LeaseGrant {
        purpose: LeasePurpose,
        /// Leader-stamped monotonic time (ms); commit advances the machine clock.
        now_ms: u64,
    },
    LeaseRenew {
        lease_id: u64,
        now_ms: u64,
    },
    LeaseRevoke {
        lease_id: u64,
    },
    /// Advance the committed monotonic clock (leader tick). Rejects backward time.
    AdvanceTime {
        now_ms: u64,
    },
    /// Advance committed time (if needed) and revoke leases due at `now_ms`.
    ExpireLeases {
        now_ms: u64,
    },
}
