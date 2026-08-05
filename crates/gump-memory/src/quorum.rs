//! Quorum rules for 1/2/3/5 voter simulations (DECISIONS D006).

use core::fmt;

/// Strict majority: `floor(n/2)+1`.
pub fn majority(voters: u32) -> Result<u32, QuorumError> {
    if voters == 0 {
        return Err(QuorumError::NoVoters);
    }
    Ok(voters / 2 + 1)
}

/// Whether `available` voters can form a commit quorum among `voters`.
///
/// D006: one node commits with itself; two require both; three tolerate one loss.
pub fn can_commit(voters: u32, available: u32) -> Result<bool, QuorumError> {
    if available > voters {
        return Err(QuorumError::AvailableExceedsVoters {
            available,
            voters,
        });
    }
    Ok(available >= majority(voters)?)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuorumError {
    NoVoters,
    AvailableExceedsVoters { available: u32, voters: u32 },
}

impl fmt::Display for QuorumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoVoters => write!(f, "voter set is empty"),
            Self::AvailableExceedsVoters { available, voters } => {
                write!(f, "available {available} exceeds voters {voters}")
            }
        }
    }
}

impl std::error::Error for QuorumError {}
