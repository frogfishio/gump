//! `AcquireController` and linearizable effect validation (PROTOCOL.md §9).

use crate::authority::fence::{EffectReject, FenceToken};
use crate::records::{LeasePurpose, LeaseTable};

/// External-effect command carrying the controller fence (and optional generation).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectCommand {
    pub token: FenceToken,
    pub declaration_generation: u64,
    pub object_revision: u64,
    pub op_id: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControllerError {
    Reject(EffectReject),
}

impl std::fmt::Display for ControllerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reject(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ControllerError {}

impl From<EffectReject> for ControllerError {
    fn from(e: EffectReject) -> Self {
        Self::Reject(e)
    }
}

/// Cluster-side controller authority record (`/authority/controller`).
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ControllerAuthority {
    current: Option<FenceToken>,
    holder: Option<u64>,
    /// Monotonic acquire counter used when deriving deterministic fences.
    acquire_nonce: u64,
}

impl ControllerAuthority {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current(&self) -> Option<&FenceToken> {
        self.current.as_ref()
    }

    pub fn holder(&self) -> Option<u64> {
        self.holder
    }

    pub fn epoch(&self) -> u64 {
        self.current.map(|t| t.epoch).unwrap_or(0)
    }

    /// `AcquireController` → commit `(epoch + 1, fence, lease_id)` under controller TTL.
    pub fn acquire(&mut self, holder: u64, now_ms: u64, leases: &mut LeaseTable) -> FenceToken {
        self.acquire_nonce = self.acquire_nonce.saturating_add(1);
        let epoch = self.epoch().saturating_add(1);
        let fence = FenceToken::derive_fence(epoch, holder, self.acquire_nonce);
        let lease = leases.grant(LeasePurpose::ControllerAuthority, now_ms);
        let token = FenceToken::new(epoch, fence, lease.id);
        self.current = Some(token);
        self.holder = Some(holder);
        token
    }

    /// Linearizable validation: presented token must match current live authority.
    pub fn validate_effect(
        &self,
        cmd: &EffectCommand,
        now_ms: u64,
        leases: &LeaseTable,
    ) -> Result<(), ControllerError> {
        let Some(current) = self.current else {
            return Err(EffectReject::NoAuthority.into());
        };
        if cmd.token.epoch < current.epoch {
            return Err(EffectReject::StaleEpoch {
                current: current.epoch,
                presented: cmd.token.epoch,
            }
            .into());
        }
        if cmd.token.epoch > current.epoch {
            // Future epoch not committed here — treat as unverifiable / stale relative to SoT.
            return Err(EffectReject::StaleEpoch {
                current: current.epoch,
                presented: cmd.token.epoch,
            }
            .into());
        }
        if cmd.token.fence != current.fence || cmd.token.lease_id != current.lease_id {
            return Err(EffectReject::FenceMismatch {
                epoch: current.epoch,
            }
            .into());
        }
        match leases.get(cmd.token.lease_id) {
            Some(lease) if lease.expires_at_ms > now_ms => Ok(()),
            _ => Err(EffectReject::ExpiredOrUnverifiable {
                lease_id: cmd.token.lease_id,
            }
            .into()),
        }
    }

    /// Accept an effect only if [`validate_effect`] succeeds. Returns the op_id on success.
    pub fn accept_effect(
        &self,
        cmd: &EffectCommand,
        now_ms: u64,
        leases: &LeaseTable,
    ) -> Result<u64, ControllerError> {
        self.validate_effect(cmd, now_ms, leases)?;
        Ok(cmd.op_id)
    }
}
