//! Membership identity types.

use core::fmt;

/// Stable node id within one cluster incarnation (PROTOCOL.md §6 / §14).
pub type MemberId = u64;

/// Cluster incarnation; changes on forced recovery (PROTOCOL.md §14).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ClusterIncarnation(pub u64);

impl ClusterIncarnation {
    pub const fn new(n: u64) -> Self {
        Self(n)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ClusterIncarnation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Lifecycle phase for a known member.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemberPhase {
    /// Authenticated; receiving RAM snapshot; never votes.
    Transferring,
    /// Snapshot verified; catching up log; still non-voting.
    Learner,
    /// Voting member of the committed configuration.
    Voter,
    /// Leaving via joint change; still counted in the outgoing set until commit.
    Draining,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberRecord {
    pub id: MemberId,
    pub phase: MemberPhase,
}
