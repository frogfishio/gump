//! `gump-memory`: RAM OpenRaft adapter (C03), typed records (C04–C05), membership (C06).
//!
//! Log, vote, membership, snapshots, and application buffers live only in RAM.
//! Typed records enforce PROTOCOL.md §6–§8. Membership covers §14 lifecycle.

#![forbid(unsafe_code)]

pub mod membership;
mod quorum;
mod ram_store;
pub mod records;

pub use membership::{
    can_commit_joint, ClusterIncarnation, JointConfig, JointError, MemberId, MemberPhase,
    MemberRecord, MembershipCluster, MembershipError, MembershipEvent, SnapshotOffer,
    SnapshotTransferError, SnapshotVerify,
};
pub use quorum::{can_commit, majority, QuorumError};
pub use ram_store::{
    ram_v2_stores, ClientRequest, ClientResponse, MemoryNodeId, RamLogStore, RamStateMachine,
    RamStore, TypeConfig,
};
pub use records::{
    comparisons_hold, ApplyError, ApplyResult, BudgetClass, BudgetError, BudgetUsage, Command,
    Compacted, Comparison, Expected, KeyPrefix, Lease, LeaseError, LeasePurpose, MemoryBudgets,
    MutateOp, RecordKey, RecordValue, Txn, TypedRecordMachine, WatchBatch, WatchChange,
    MAX_WATCH_AGE_MS, MAX_WATCH_REVISIONS,
};
