//! Joint consensus configuration (DECISIONS D006 / CONFORMANCE joint-membership).

use std::collections::BTreeSet;

use crate::membership::types::MemberId;
use crate::quorum::{can_commit, QuorumError};

/// Overlapping old and new voter sets during a membership change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JointConfig {
    pub old_voters: BTreeSet<MemberId>,
    pub new_voters: BTreeSet<MemberId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JointError {
    Quorum(QuorumError),
    EmptyOld,
    EmptyNew,
}

impl std::fmt::Display for JointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Quorum(e) => write!(f, "{e}"),
            Self::EmptyOld => write!(f, "joint old voter set is empty"),
            Self::EmptyNew => write!(f, "joint new voter set is empty"),
        }
    }
}

impl std::error::Error for JointError {}

impl From<QuorumError> for JointError {
    fn from(e: QuorumError) -> Self {
        Self::Quorum(e)
    }
}

impl JointConfig {
    pub fn new(
        old_voters: BTreeSet<MemberId>,
        new_voters: BTreeSet<MemberId>,
    ) -> Result<Self, JointError> {
        if old_voters.is_empty() {
            return Err(JointError::EmptyOld);
        }
        if new_voters.is_empty() {
            return Err(JointError::EmptyNew);
        }
        Ok(Self {
            old_voters,
            new_voters,
        })
    }

    pub fn intersection(&self) -> BTreeSet<MemberId> {
        self.old_voters
            .intersection(&self.new_voters)
            .copied()
            .collect()
    }
}

/// Joint commit requires a majority of the old set **and** a majority of the new set
/// among the available voters (CONFORMANCE: joint-membership intersection).
pub fn can_commit_joint(
    joint: &JointConfig,
    available: &BTreeSet<MemberId>,
) -> Result<bool, JointError> {
    let old_n = joint.old_voters.len() as u32;
    let new_n = joint.new_voters.len() as u32;
    let old_avail = joint.old_voters.intersection(available).count() as u32;
    let new_avail = joint.new_voters.intersection(available).count() as u32;
    Ok(can_commit(old_n, old_avail)? && can_commit(new_n, new_avail)?)
}
