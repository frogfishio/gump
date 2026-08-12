//! `gump-memory`: RAM OpenRaft (C03), records (C04–C05), membership (C06), authority (C07).
//!
//! Log, vote, membership, snapshots, and application buffers live only in RAM.
//! Controller fencing: PROTOCOL.md §9 / CONFORMANCE INV-007.
//! STL-01: application mutations apply through [`cluster_state::ClusterState`] via
//! [`cluster_state::RaftCommand`]; OpenRaft `StoredMembership` owns voter sets.

#![forbid(unsafe_code)]

pub mod authority;
pub mod cluster_net;
pub mod cluster_node;
pub mod cluster_state;
pub mod membership;
mod quorum;
mod ram_store;
pub mod records;

pub use authority::{
    AgentFenceError, AgentFenceMemory, ControllerAuthority, ControllerError, EffectCommand,
    EffectReject, FenceToken,
};
pub use cluster_net::{ClusterJoinConfig, ClusterNetworkConfig};
pub use cluster_node::{ClusterStatusSnapshot, ControlSnapshot, MemoryCluster};
pub use cluster_state::{
    ApplyOutcome, ClusterState, DESIRED_MAX_ENTRIES, DESIRED_MAX_PAYLOAD_BYTES,
    DESIRED_MAX_TOTAL_BYTES, DesiredSnapshotEntry, FINITE_COMPLETION_MAX_ENTRIES,
    IDEMPOTENCY_MAX_ENTRIES, IDEMPOTENCY_TTL_MS, RaftCommand, RaftResponse,
};
pub use membership::{
    ClusterIncarnation, JointConfig, JointError, MemberId, MemberPhase, MemberRecord,
    MembershipCluster, MembershipError, MembershipEvent, SnapshotOffer, SnapshotTransferError,
    SnapshotVerify, can_commit_joint,
};
pub use quorum::{QuorumError, can_commit, majority};
pub use ram_store::{
    ClientRequest, ClientResponse, MemoryNodeId, RamLogStore, RamStateMachine, RamStore,
    TypeConfig, ram_v2_stores,
};
pub use records::{
    ApplyError, ApplyResult, BudgetClass, BudgetError, BudgetUsage, Command, Compacted, Comparison,
    Expected, KeyPrefix, Lease, LeaseError, LeasePurpose, LeaseTable, MAX_WATCH_AGE_MS,
    MAX_WATCH_REVISIONS, MemoryBudgets, MutateOp, RecordKey, RecordValue, Txn, TypedRecordMachine,
    WatchBatch, WatchChange, comparisons_hold,
};
