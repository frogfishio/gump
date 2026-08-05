//! Certificate rotation: open a new session and drain the old (PROTOCOL.md §3).

use crate::identity::TransportIdentity;

/// Local view of concurrent sessions during certificate rotation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionSlot {
    Active {
        identity: TransportIdentity,
        generation: u64,
    },
    Draining {
        identity: TransportIdentity,
        generation: u64,
        successor: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RotationPlan {
    pub previous: SessionSlot,
    pub next: SessionSlot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RotationAction {
    /// Start draining the previous generation; new traffic uses successor.
    BeginDrain { from: u64, to: u64 },
    /// Previous generation fully drained; only successor remains.
    Complete { active: u64 },
    /// Rejected: successor identity must match cluster, bump incarnation.
    Rejected(&'static str),
}

impl SessionSlot {
    pub fn active(identity: TransportIdentity, generation: u64) -> Self {
        Self::Active {
            identity,
            generation,
        }
    }

    pub fn generation(&self) -> u64 {
        match self {
            Self::Active { generation, .. } | Self::Draining { generation, .. } => *generation,
        }
    }

    pub fn identity(&self) -> &TransportIdentity {
        match self {
            Self::Active { identity, .. } | Self::Draining { identity, .. } => identity,
        }
    }

    /// Begin rotation to `new_identity` under `new_generation`.
    pub fn begin_rotation(
        &self,
        new_identity: TransportIdentity,
        new_generation: u64,
    ) -> Result<(RotationPlan, RotationAction), RotationAction> {
        let Self::Active {
            identity,
            generation,
        } = self
        else {
            return Err(RotationAction::Rejected("already draining"));
        };
        if new_generation <= *generation {
            return Err(RotationAction::Rejected("generation must increase"));
        }
        if new_identity.cluster_id != identity.cluster_id {
            return Err(RotationAction::Rejected("cluster mismatch"));
        }
        if new_identity.node_id != identity.node_id {
            return Err(RotationAction::Rejected("node mismatch"));
        }
        // Rotation issues a new short-lived cert; incarnation may change.
        let plan = RotationPlan {
            previous: SessionSlot::Draining {
                identity: identity.clone(),
                generation: *generation,
                successor: new_generation,
            },
            next: SessionSlot::Active {
                identity: new_identity,
                generation: new_generation,
            },
        };
        Ok((
            plan,
            RotationAction::BeginDrain {
                from: *generation,
                to: new_generation,
            },
        ))
    }

    /// Finish drain once the previous generation has no live streams.
    pub fn complete_drain(plan: &RotationPlan) -> (SessionSlot, RotationAction) {
        let SessionSlot::Active {
            identity,
            generation,
        } = &plan.next
        else {
            // next is always Active by construction
            unreachable!("rotation next slot is Active");
        };
        (
            SessionSlot::Active {
                identity: identity.clone(),
                generation: *generation,
            },
            RotationAction::Complete {
                active: *generation,
            },
        )
    }
}
