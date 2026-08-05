//! `gump-memory`: RAM OpenRaft adapter (C03) and typed record SM (C04–C05).
//!
//! Log, vote, membership, snapshots, and application buffers live only in RAM.
//! Typed records enforce PROTOCOL.md §6–§8 commands, budgets, watches, and leases.

#![forbid(unsafe_code)]

mod quorum;
mod ram_store;
pub mod records;

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
