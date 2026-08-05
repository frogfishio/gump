//! Cluster membership lifecycle (C06 / PROTOCOL.md §14 / DECISIONS D006).
//!
//! Init → join as non-voting learner → RAM snapshot transfer → catch-up →
//! joint-consensus promotion; drain and remove. Seed has no special role after join.

mod joint;
mod lifecycle;
mod snapshot;
mod types;

pub use joint::{can_commit_joint, JointConfig, JointError};
pub use lifecycle::{MembershipCluster, MembershipError, MembershipEvent};
pub use snapshot::{SnapshotOffer, SnapshotTransferError, SnapshotVerify};
pub use types::{ClusterIncarnation, MemberId, MemberPhase, MemberRecord};
