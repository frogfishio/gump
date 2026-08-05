//! `gump-memory`: RAM OpenRaft (C03), records (C04–C05), membership (C06), authority (C07).
//!
//! Log, vote, membership, snapshots, and application buffers live only in RAM.
//! Controller fencing: PROTOCOL.md §9 / CONFORMANCE INV-007.

#![forbid(unsafe_code)]

pub mod authority;
pub mod membership;
mod quorum;
mod ram_store;
pub mod records;

pub use authority::{
    AgentFenceError, AgentFenceMemory, ControllerAuthority, ControllerError, EffectCommand,
    EffectReject, FenceToken,
};
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
    Compacted, Comparison, Expected, KeyPrefix, Lease, LeaseError, LeasePurpose, LeaseTable,
    MemoryBudgets, MutateOp, RecordKey, RecordValue, Txn, TypedRecordMachine, WatchBatch,
    WatchChange, MAX_WATCH_AGE_MS, MAX_WATCH_REVISIONS,
};
