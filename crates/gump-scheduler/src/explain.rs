//! Stable explain reason codes for hard-filter / admission (RUNTIME.md §2).

/// One rejection or score component with a stable code for `gump explain`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExplainReason {
    pub code: &'static str,
    /// Bounded numeric evidence (never secrets).
    pub evidence: i64,
    pub detail: String,
}

impl ExplainReason {
    pub fn new(code: &'static str, evidence: i64, detail: impl Into<String>) -> Self {
        Self {
            code,
            evidence,
            detail: detail.into(),
        }
    }
}

/// Stable hard-filter / admission codes (do not rename without a CONFORMANCE bump).
pub mod codes {
    pub const ARCH_MISMATCH: &str = "hard.arch_mismatch";
    pub const DRIVER_MISSING: &str = "hard.driver_missing";
    pub const CAPABILITY_MISSING: &str = "hard.capability_missing";
    pub const CAPABILITY_NOT_ENFORCED: &str = "hard.capability_not_enforced";
    pub const MILLICORES: &str = "hard.millicores_insufficient";
    pub const MEMORY: &str = "hard.memory_insufficient";
    pub const GPU: &str = "hard.gpu_insufficient";
    pub const PORT_REQUIRED: &str = "hard.port_required";
    pub const NODE_DRAINED: &str = "hard.node_drained";
    pub const STALE_CAPABILITY: &str = "hard.stale_capability";
    pub const STALE_FENCE: &str = "hard.stale_fence";
    pub const LEDGER_FULL: &str = "hard.ledger_full";
    pub const NO_CANDIDATE: &str = "hard.no_feasible_candidate";
    pub const SCORE_HEADROOM: &str = "score.residual_headroom";
}
