//! Agent-local accepted fence memory (PROTOCOL.md §9).
//!
//! A higher epoch permanently fences lower epochs for that process lifetime.
//! Equal epoch with a different fence is a protocol violation.

use crate::authority::fence::{EffectReject, FenceToken};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentFenceError {
    Reject(EffectReject),
    /// Equal epoch, different fence — must not be silently accepted.
    ProtocolViolation {
        epoch: u64,
    },
}

impl std::fmt::Display for AgentFenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reject(e) => write!(f, "{e}"),
            Self::ProtocolViolation { epoch } => {
                write!(f, "protocol violation: conflicting fence at epoch {epoch}")
            }
        }
    }
}

impl std::error::Error for AgentFenceError {}

/// In-memory only: the fence an agent has accepted for local effects.
#[derive(Clone, Debug, Default)]
pub struct AgentFenceMemory {
    accepted: Option<FenceToken>,
    /// Highest epoch ever observed; lower epochs stay fenced for process lifetime.
    max_epoch: u64,
}

impl AgentFenceMemory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn accepted(&self) -> Option<&FenceToken> {
        self.accepted.as_ref()
    }

    pub fn max_epoch(&self) -> u64 {
        self.max_epoch
    }

    /// Accept a new controller fence from a validated authority proof / acquire.
    pub fn accept(&mut self, token: FenceToken) -> Result<(), AgentFenceError> {
        if token.epoch < self.max_epoch {
            return Err(AgentFenceError::Reject(EffectReject::StaleEpoch {
                current: self.max_epoch,
                presented: token.epoch,
            }));
        }
        if token.epoch == self.max_epoch {
            if let Some(prev) = &self.accepted {
                if prev.fence != token.fence {
                    return Err(AgentFenceError::ProtocolViolation {
                        epoch: token.epoch,
                    });
                }
            }
        }
        self.max_epoch = self.max_epoch.max(token.epoch);
        self.accepted = Some(token);
        Ok(())
    }

    /// Whether this agent may perform a local effect under `token`.
    pub fn authorize_effect(&self, token: &FenceToken) -> Result<(), AgentFenceError> {
        if token.epoch < self.max_epoch {
            return Err(AgentFenceError::Reject(EffectReject::StaleEpoch {
                current: self.max_epoch,
                presented: token.epoch,
            }));
        }
        let Some(accepted) = &self.accepted else {
            return Err(AgentFenceError::Reject(EffectReject::NoAuthority));
        };
        if token.epoch != accepted.epoch {
            return Err(AgentFenceError::Reject(EffectReject::StaleEpoch {
                current: accepted.epoch,
                presented: token.epoch,
            }));
        }
        if token.fence != accepted.fence {
            return Err(AgentFenceError::ProtocolViolation {
                epoch: token.epoch,
            });
        }
        Ok(())
    }
}
