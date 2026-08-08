//! Lifecycle checks, retry/backoff, and terminal reasons (RUNTIME.md §9 / §11 / R09).
//!
//! Health is optional and never implies a workload type. Readiness and
//! publication eligibility are never inferred when undeclared.

/// Default exponential backoff bounds (RUNTIME.md §9).
pub const DEFAULT_INITIAL_BACKOFF_MS: u64 = 1_000;
pub const DEFAULT_MAX_BACKOFF_MS: u64 = 5 * 60 * 1_000;
pub const DEFAULT_JITTER_PCT: u32 = 20;

/// Stable, bounded terminal / transition reason codes for `gump explain`.
pub mod reasons {
    pub const COMPLETED: &str = "lifecycle.completed";
    pub const PERMANENT_FAILURE: &str = "lifecycle.permanent_failure";
    pub const RETRY_SCHEDULED: &str = "lifecycle.retry_scheduled";
    pub const LIVENESS_FAILED: &str = "lifecycle.liveness_failed";
    pub const READINESS_FAILED: &str = "lifecycle.readiness_failed";
    pub const CHECK_TIMEOUT: &str = "lifecycle.check_timeout";
    pub const CHECK_SKIPPED_BUDGET: &str = "lifecycle.check_skipped_budget";
    pub const STOP_SIGNAL: &str = "lifecycle.stop_signal";
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckKind {
    /// Process still running (OS observation).
    Process,
    /// TCP connect to `host:port`.
    Tcp,
    /// Minimal HTTP GET; success = status in 200..399.
    Http,
    /// Run argv under a timeout (attempt isolation; no undeclared secrets).
    Command,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckSpec {
    pub kind: CheckKind,
    /// For Tcp/Http: `host:port` or `http://host:port/path`.
    pub target: Option<String>,
    /// For Command: argv (absolute or release-relative resolved by caller).
    pub command: Option<Vec<String>>,
    pub interval_ms: u64,
    pub timeout_ms: u64,
    pub initial_delay_ms: u64,
    pub success_threshold: u32,
    pub failure_threshold: u32,
    pub max_output_bytes: usize,
}

impl CheckSpec {
    pub fn process_default() -> Self {
        Self {
            kind: CheckKind::Process,
            target: None,
            command: None,
            interval_ms: 1_000,
            timeout_ms: 500,
            initial_delay_ms: 0,
            success_threshold: 1,
            failure_threshold: 1,
            max_output_bytes: 4_096,
        }
    }
}

/// Declared retry policy. Absent / `max_attempts == 0` means no retry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub jitter_pct: u32,
    pub reset_window_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 0,
            initial_backoff_ms: DEFAULT_INITIAL_BACKOFF_MS,
            max_backoff_ms: DEFAULT_MAX_BACKOFF_MS,
            jitter_pct: DEFAULT_JITTER_PCT,
            reset_window_ms: 10 * 60 * 1_000,
        }
    }
}

impl RetryPolicy {
    pub fn enabled(&self) -> bool {
        self.max_attempts > 0
    }

    /// Deterministic backoff with bounded jitter from `attempt_index` (1-based failure count).
    pub fn backoff_ms(&self, attempt_index: u32, jitter_seed: u64) -> u64 {
        if attempt_index == 0 {
            return 0;
        }
        let exp = attempt_index.saturating_sub(1).min(16);
        let base = self
            .initial_backoff_ms
            .saturating_mul(1u64 << exp)
            .min(self.max_backoff_ms);
        if self.jitter_pct == 0 {
            return base;
        }
        let span = base.saturating_mul(u64::from(self.jitter_pct)) / 100;
        let jitter = if span == 0 {
            0
        } else {
            jitter_seed % (span.saturating_add(1))
        };
        base.saturating_sub(span / 2).saturating_add(jitter)
    }
}

/// Full lifecycle contract attached to an accepted placement (optional health).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LifecycleContract {
    pub readiness: Option<CheckSpec>,
    pub liveness: Option<CheckSpec>,
    pub completion: Option<CheckSpec>,
    pub retry: RetryPolicy,
    /// When true, publication may be considered after readiness (never inferred).
    pub declares_publication: bool,
    /// Stop signal preference for graceful terminate (Term vs Kill).
    pub stop_grace_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalReason {
    pub code: &'static str,
    pub detail: String,
    pub exit_code: Option<i32>,
    pub attempt_index: u32,
}

impl TerminalReason {
    pub fn completed(exit: Option<i32>) -> Self {
        Self {
            code: reasons::COMPLETED,
            detail: "finite workload completed".into(),
            exit_code: exit,
            attempt_index: 1,
        }
    }

    pub fn permanent(detail: impl Into<String>, exit: Option<i32>, attempt_index: u32) -> Self {
        Self {
            code: reasons::PERMANENT_FAILURE,
            detail: detail.into(),
            exit_code: exit,
            attempt_index,
        }
    }
}

/// Per-check runtime counters (bounded; no secrets).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CheckRuntime {
    pub consecutive_success: u32,
    pub consecutive_failure: u32,
    pub last_run_ms: u64,
    pub passed: bool,
}

impl CheckRuntime {
    pub fn record(&mut self, ok: bool, spec: &CheckSpec) {
        if ok {
            self.consecutive_success = self.consecutive_success.saturating_add(1);
            self.consecutive_failure = 0;
            if self.consecutive_success >= spec.success_threshold.max(1) {
                self.passed = true;
            }
        } else {
            self.consecutive_failure = self.consecutive_failure.saturating_add(1);
            self.consecutive_success = 0;
            if self.consecutive_failure >= spec.failure_threshold.max(1) {
                self.passed = false;
            }
        }
    }

    pub fn due(&self, now_ms: u64, started_ms: u64, spec: &CheckSpec) -> bool {
        if now_ms.saturating_sub(started_ms) < spec.initial_delay_ms {
            return false;
        }
        now_ms.saturating_sub(self.last_run_ms) >= spec.interval_ms
    }
}
