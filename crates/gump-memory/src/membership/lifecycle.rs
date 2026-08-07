//! Membership state machine: init / join / promote / drain / remove.
//!
//! **STL-01 authority split:** OpenRaft `StoredMembership` is the sole source of
//! truth for which node IDs are voters/learners for log commit. This
//! [`MembershipCluster`] tracks *application* member phases (transfer, drain,
//! incarnation metadata). Do not use [`MembershipCluster::voters`] or
//! [`crate::membership::can_commit_joint`] to decide whether a Raft log entry
//! may commit — that remains OpenRaft's job. Joint config helpers here only
//! validate set arithmetic for tests / operator preflight.

use std::collections::{BTreeMap, BTreeSet};

use crate::membership::joint::{JointConfig, can_commit_joint};
use crate::membership::snapshot::{SnapshotOffer, SnapshotTransferError, SnapshotVerify};
use crate::membership::types::{ClusterIncarnation, MemberId, MemberPhase, MemberRecord};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MembershipError {
    AlreadyInitialized,
    NotInitialized,
    MemberExists(MemberId),
    UnknownMember(MemberId),
    BadPhase {
        id: MemberId,
        phase: MemberPhase,
        needed: &'static str,
    },
    JointInProgress,
    NoJoint,
    Snapshot(SnapshotTransferError),
    Joint(crate::membership::joint::JointError),
    JointQuorumNotMet,
    NotAVoter(MemberId),
    LastVoter,
}

impl std::fmt::Display for MembershipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyInitialized => write!(f, "cluster already initialized"),
            Self::NotInitialized => write!(f, "cluster not initialized"),
            Self::MemberExists(id) => write!(f, "member {id} already exists"),
            Self::UnknownMember(id) => write!(f, "unknown member {id}"),
            Self::BadPhase { id, phase, needed } => {
                write!(f, "member {id} in phase {phase:?}; needed {needed}")
            }
            Self::JointInProgress => write!(f, "joint membership change already in progress"),
            Self::NoJoint => write!(f, "no joint membership change in progress"),
            Self::Snapshot(e) => write!(f, "{e}"),
            Self::Joint(e) => write!(f, "{e}"),
            Self::JointQuorumNotMet => {
                write!(
                    f,
                    "joint commit requires majority of old and new voter sets"
                )
            }
            Self::NotAVoter(id) => write!(f, "member {id} is not a voter"),
            Self::LastVoter => write!(f, "cannot drain/remove the last voter"),
        }
    }
}

impl std::error::Error for MembershipError {}

impl From<SnapshotTransferError> for MembershipError {
    fn from(e: SnapshotTransferError) -> Self {
        Self::Snapshot(e)
    }
}

impl From<crate::membership::joint::JointError> for MembershipError {
    fn from(e: crate::membership::joint::JointError) -> Self {
        Self::Joint(e)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MembershipEvent {
    Initialized {
        seed: MemberId,
        incarnation: ClusterIncarnation,
    },
    JoinStarted {
        id: MemberId,
    },
    TransferCompleted {
        id: MemberId,
        committed_index: u64,
    },
    TransferAborted {
        id: MemberId,
    },
    JointEntered {
        joint: JointConfig,
    },
    JointCommitted {
        voters: BTreeSet<MemberId>,
    },
    Removed {
        id: MemberId,
    },
}

/// Authoritative membership view for one cluster incarnation (RAM-only).
#[derive(Clone, Debug, Default)]
pub struct MembershipCluster {
    incarnation: Option<ClusterIncarnation>,
    members: BTreeMap<MemberId, MemberRecord>,
    voters: BTreeSet<MemberId>,
    learners: BTreeSet<MemberId>,
    joint: Option<JointConfig>,
    /// Expected snapshot index advertised by the cluster during a transfer.
    transfer_index: BTreeMap<MemberId, u64>,
    transfer_digest: BTreeMap<MemberId, [u8; 32]>,
}

impl MembershipCluster {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn incarnation(&self) -> Option<ClusterIncarnation> {
        self.incarnation
    }

    pub fn voters(&self) -> &BTreeSet<MemberId> {
        &self.voters
    }

    pub fn learners(&self) -> &BTreeSet<MemberId> {
        &self.learners
    }

    pub fn joint(&self) -> Option<&JointConfig> {
        self.joint.as_ref()
    }

    pub fn member(&self, id: MemberId) -> Option<&MemberRecord> {
        self.members.get(&id)
    }

    pub fn is_voter(&self, id: MemberId) -> bool {
        self.voters.contains(&id)
    }

    /// True only for committed voters (not Transferring / Learner / Draining alone).
    pub fn can_vote(&self, id: MemberId) -> bool {
        matches!(
            self.members.get(&id).map(|m| m.phase),
            Some(MemberPhase::Voter) | Some(MemberPhase::Draining)
        ) && self.voters.contains(&id)
    }

    /// First server: `server --init`.
    pub fn init(
        &mut self,
        seed: MemberId,
        incarnation: ClusterIncarnation,
    ) -> Result<MembershipEvent, MembershipError> {
        if self.incarnation.is_some() {
            return Err(MembershipError::AlreadyInitialized);
        }
        self.incarnation = Some(incarnation);
        self.members.insert(
            seed,
            MemberRecord {
                id: seed,
                phase: MemberPhase::Voter,
            },
        );
        self.voters.insert(seed);
        Ok(MembershipEvent::Initialized { seed, incarnation })
    }

    /// Begin join as non-voting learner transferring RAM state.
    pub fn begin_join(
        &mut self,
        id: MemberId,
        expected_index: u64,
        expected_digest: [u8; 32],
    ) -> Result<MembershipEvent, MembershipError> {
        self.require_init()?;
        if self.members.contains_key(&id) {
            return Err(MembershipError::MemberExists(id));
        }
        if self.joint.is_some() {
            return Err(MembershipError::JointInProgress);
        }
        self.members.insert(
            id,
            MemberRecord {
                id,
                phase: MemberPhase::Transferring,
            },
        );
        self.learners.insert(id);
        self.transfer_index.insert(id, expected_index);
        self.transfer_digest.insert(id, expected_digest);
        Ok(MembershipEvent::JoinStarted { id })
    }

    /// Verify snapshot digest/index; promote Transferring → Learner (still non-voting).
    pub fn complete_transfer(
        &mut self,
        id: MemberId,
        offer: &SnapshotOffer,
    ) -> Result<MembershipEvent, MembershipError> {
        self.require_init()?;
        let rec = self
            .members
            .get_mut(&id)
            .ok_or(MembershipError::UnknownMember(id))?;
        if rec.phase != MemberPhase::Transferring {
            return Err(MembershipError::BadPhase {
                id,
                phase: rec.phase,
                needed: "Transferring",
            });
        }
        let expected_index = *self
            .transfer_index
            .get(&id)
            .ok_or(MembershipError::UnknownMember(id))?;
        let expected_digest = *self
            .transfer_digest
            .get(&id)
            .ok_or(MembershipError::UnknownMember(id))?;
        offer.verify(expected_index, expected_digest)?;
        rec.phase = MemberPhase::Learner;
        self.transfer_index.remove(&id);
        self.transfer_digest.remove(&id);
        Ok(MembershipEvent::TransferCompleted {
            id,
            committed_index: offer.committed_index,
        })
    }

    /// Joiner crash / abort during transfer: never votes; no authority gained.
    pub fn abort_transfer(&mut self, id: MemberId) -> Result<MembershipEvent, MembershipError> {
        self.require_init()?;
        let rec = self
            .members
            .get(&id)
            .ok_or(MembershipError::UnknownMember(id))?;
        if rec.phase != MemberPhase::Transferring {
            return Err(MembershipError::BadPhase {
                id,
                phase: rec.phase,
                needed: "Transferring",
            });
        }
        self.members.remove(&id);
        self.learners.remove(&id);
        self.transfer_index.remove(&id);
        self.transfer_digest.remove(&id);
        debug_assert!(!self.voters.contains(&id));
        Ok(MembershipEvent::TransferAborted { id })
    }

    /// Enter joint consensus to add a caught-up learner as voter.
    pub fn begin_promote(&mut self, id: MemberId) -> Result<MembershipEvent, MembershipError> {
        self.require_init()?;
        if self.joint.is_some() {
            return Err(MembershipError::JointInProgress);
        }
        let rec = self
            .members
            .get(&id)
            .ok_or(MembershipError::UnknownMember(id))?;
        if rec.phase != MemberPhase::Learner {
            return Err(MembershipError::BadPhase {
                id,
                phase: rec.phase,
                needed: "Learner",
            });
        }
        let mut new_voters = self.voters.clone();
        new_voters.insert(id);
        let joint = JointConfig::new(self.voters.clone(), new_voters)?;
        self.joint = Some(joint.clone());
        Ok(MembershipEvent::JointEntered { joint })
    }

    /// Enter joint consensus to drain a voter (outgoing config drops them).
    pub fn begin_drain(&mut self, id: MemberId) -> Result<MembershipEvent, MembershipError> {
        self.require_init()?;
        if self.joint.is_some() {
            return Err(MembershipError::JointInProgress);
        }
        if !self.voters.contains(&id) {
            return Err(MembershipError::NotAVoter(id));
        }
        if self.voters.len() == 1 {
            return Err(MembershipError::LastVoter);
        }
        let rec = self
            .members
            .get_mut(&id)
            .ok_or(MembershipError::UnknownMember(id))?;
        if rec.phase != MemberPhase::Voter {
            return Err(MembershipError::BadPhase {
                id,
                phase: rec.phase,
                needed: "Voter",
            });
        }
        rec.phase = MemberPhase::Draining;
        let mut new_voters = self.voters.clone();
        new_voters.remove(&id);
        let joint = JointConfig::new(self.voters.clone(), new_voters)?;
        self.joint = Some(joint.clone());
        Ok(MembershipEvent::JointEntered { joint })
    }

    /// Commit the joint configuration when both majorities acknowledge.
    pub fn commit_joint(
        &mut self,
        available: &BTreeSet<MemberId>,
    ) -> Result<MembershipEvent, MembershipError> {
        self.require_init()?;
        let joint = self.joint.take().ok_or(MembershipError::NoJoint)?;
        if !can_commit_joint(&joint, available)? {
            self.joint = Some(joint);
            return Err(MembershipError::JointQuorumNotMet);
        }

        let new_voters = joint.new_voters.clone();
        let leaving: Vec<MemberId> = joint.old_voters.difference(&new_voters).copied().collect();
        let joining: Vec<MemberId> = new_voters.difference(&joint.old_voters).copied().collect();

        for id in &joining {
            if let Some(rec) = self.members.get_mut(id) {
                rec.phase = MemberPhase::Voter;
            }
            self.learners.remove(id);
        }
        for id in &leaving {
            self.members.remove(id);
            self.learners.remove(id);
        }
        self.voters = new_voters.clone();
        Ok(MembershipEvent::JointCommitted { voters: new_voters })
    }

    /// Force-remove a non-voter (e.g. leftover learner); voters must drain via joint first.
    pub fn remove(&mut self, id: MemberId) -> Result<MembershipEvent, MembershipError> {
        self.require_init()?;
        if self.joint.is_some() {
            return Err(MembershipError::JointInProgress);
        }
        if self.voters.contains(&id) {
            return Err(MembershipError::BadPhase {
                id,
                phase: MemberPhase::Voter,
                needed: "non-voter (drain via joint first)",
            });
        }
        if !self.members.contains_key(&id) {
            return Err(MembershipError::UnknownMember(id));
        }
        self.members.remove(&id);
        self.learners.remove(&id);
        self.transfer_index.remove(&id);
        self.transfer_digest.remove(&id);
        Ok(MembershipEvent::Removed { id })
    }

    fn require_init(&self) -> Result<(), MembershipError> {
        if self.incarnation.is_none() {
            Err(MembershipError::NotInitialized)
        } else {
            Ok(())
        }
    }
}
