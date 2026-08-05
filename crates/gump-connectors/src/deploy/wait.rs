//! Deploy wait conditions (DECISIONS D014 / CLI_LIFECYCLE.md).

use crate::deploy::types::WorkloadContract;

/// Explicit `gump deploy --wait` conditions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitCondition {
    /// Capsule persisted and live intent accepted.
    Accepted,
    /// At least one unit started.
    Started,
    /// Declared readiness / eligibility satisfied.
    Eligible,
    /// Publication provider reported success.
    Published,
    /// Finite execution completed successfully.
    Completed,
}

impl WaitCondition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Started => "started",
            Self::Eligible => "eligible",
            Self::Published => "published",
            Self::Completed => "completed",
        }
    }

    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "accepted" => Some(Self::Accepted),
            "started" => Some(Self::Started),
            "eligible" => Some(Self::Eligible),
            "published" => Some(Self::Published),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }
}

/// Default wait derived from the declared workload contract (D014).
///
/// A finite non-networked job never waits for readiness or publication it did
/// not declare (CLI_LIFECYCLE.md invariant 7).
pub fn default_wait_condition(contract: &WorkloadContract) -> WaitCondition {
    if contract.lifecycle_finite {
        return WaitCondition::Completed;
    }
    if contract.requires_publication {
        return WaitCondition::Published;
    }
    if contract.declares_readiness {
        return WaitCondition::Eligible;
    }
    WaitCondition::Started
}
