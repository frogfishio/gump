//! In-memory Raft storage (DELIVERY C03 / DECISIONS D001, D006).
//!
//! Log, vote, membership, snapshots, and application buffers live only in RAM.
//! The v2 surface is exposed through [`openraft::storage::Adaptor`].

#![forbid(unsafe_code)]

mod quorum;
mod ram_store;

pub use quorum::{can_commit, majority, QuorumError};
pub use ram_store::{
    ram_v2_stores, ClientRequest, ClientResponse, MemoryNodeId, RamLogStore, RamStateMachine,
    RamStore, TypeConfig,
};
