//! Fenced authority and isolation grace (RUNTIME.md §10 / R10 / INV-014).

/// Default isolation grace: 15 minutes (DECISIONS / RUNTIME.md §10).
pub const DEFAULT_ISOLATION_GRACE_MS: u64 = 15 * 60 * 1_000;
/// Short confirmation window when `stop_on_isolation` is set.
pub const STOP_ON_ISOLATION_CONFIRM_MS: u64 = 1_000;

/// Live controller/placement authority as known to the agent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorityState {
    pub controller_epoch: u64,
    pub placement_fence: u64,
    /// When set, the agent cannot validate current authority (partition).
    pub isolated_since_ms: Option<u64>,
}

impl AuthorityState {
    pub fn connected(controller_epoch: u64, placement_fence: u64) -> Self {
        Self {
            controller_epoch,
            placement_fence,
            isolated_since_ms: None,
        }
    }

    pub fn is_isolated(&self) -> bool {
        self.isolated_since_ms.is_some()
    }
}

/// Declared isolation policy for this agent / workload class.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IsolationPolicy {
    pub grace_ms: u64,
    pub stop_on_isolation: bool,
    pub confirm_window_ms: u64,
}

impl Default for IsolationPolicy {
    fn default() -> Self {
        Self {
            grace_ms: DEFAULT_ISOLATION_GRACE_MS,
            stop_on_isolation: false,
            confirm_window_ms: STOP_ON_ISOLATION_CONFIRM_MS,
        }
    }
}

impl IsolationPolicy {
    pub fn effective_grace_ms(&self) -> u64 {
        if self.stop_on_isolation {
            self.confirm_window_ms
        } else {
            self.grace_ms
        }
    }
}

/// Effect classes gated by fence + isolation (INV-014).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectKind {
    Start,
    Restart,
    Stop,
    Publish,
    Refresh,
    Report,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FenceError {
    StaleFence { live: u64, expected: u64 },
    Isolated { effect: EffectKind },
    GraceExpired,
}

impl std::fmt::Display for FenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StaleFence { live, expected } => {
                write!(f, "stale placement fence: live={live} expected={expected}")
            }
            Self::Isolated { effect } => {
                write!(f, "isolated: {:?} effect forbidden", effect)
            }
            Self::GraceExpired => write!(f, "isolation grace expired"),
        }
    }
}

impl std::error::Error for FenceError {}

/// Require `expected_fence` to match live authority fence exactly.
pub fn require_fence(authority: &AuthorityState, expected_fence: u64) -> Result<(), FenceError> {
    if authority.placement_fence != expected_fence {
        return Err(FenceError::StaleFence {
            live: authority.placement_fence,
            expected: expected_fence,
        });
    }
    Ok(())
}

/// Effects while isolated: only continuing an already-running attempt is allowed.
/// Start/restart/stop/publish/refresh/report are denied (RUNTIME.md §10, INV-014).
pub fn allow_effect(
    authority: &AuthorityState,
    expected_fence: u64,
    effect: EffectKind,
) -> Result<(), FenceError> {
    require_fence(authority, expected_fence)?;
    if authority.is_isolated() {
        return Err(FenceError::Isolated { effect });
    }
    Ok(())
}

/// Whether isolation grace has expired (workload must terminate + clean).
pub fn isolation_grace_expired(
    authority: &AuthorityState,
    policy: &IsolationPolicy,
    now_ms: u64,
) -> bool {
    match authority.isolated_since_ms {
        None => false,
        Some(since) => now_ms.saturating_sub(since) >= policy.effective_grace_ms(),
    }
}
