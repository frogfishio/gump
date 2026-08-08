//! Ownership boundary for `gump-agent` (see docs/v1/README.md §5).
//!
//! Process hardening for secret custody is applied at agent startup (STL-20 /
//! SECURITY.md §8) before any future enrollment or Capsule materialization path
//! holds plaintext secrets.
//!
//! Scoped secret delivery (GUMP-N009 / S07) lives in [`delivery`].
//! Fenced reconcile / supervision (GUMP-N012 / R06) lives in [`reconcile`].

mod delivery;
mod fence;
mod reconcile;

use gump_types::{
    HardenError, HardenPolicy, ProcessHardenReport, prepare_for_custody_with_policy,
    prepare_service_for_custody,
};

pub use delivery::{DeliveryError, authorize_delivery, bind_secret_plan};
pub use fence::{
    AuthorityState, DEFAULT_ISOLATION_GRACE_MS, EffectKind, FenceError, IsolationPolicy,
    STOP_ON_ISOLATION_CONFIRM_MS, allow_effect, isolation_grace_expired, require_fence,
};
pub use reconcile::{
    AcceptedPlacement, AgentError, AttemptPhase, AttemptReport, DEFAULT_MAX_LIVE_ATTEMPTS,
    EffectExecutor,
};

/// Early agent process hardening. Uses [`prepare_service_for_custody`] (policy
/// `required` by default; override with `GUMP_PROCESS_HARDEN`).
pub fn harden_agent_startup() -> Result<ProcessHardenReport, HardenError> {
    prepare_service_for_custody()
}

/// Explicit-policy variant for tests and privileged deployments.
pub fn harden_agent_startup_with_policy(
    policy: HardenPolicy,
) -> Result<ProcessHardenReport, HardenError> {
    prepare_for_custody_with_policy(policy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_startup_hardens_without_sealed_builder() {
        let report = harden_agent_startup().expect("service Required policy");
        assert!(report.panic_hook_installed);
        #[cfg(unix)]
        assert!(report.core_dumps_disabled);
        // Surface status string for operators / logs.
        let status = report.to_string();
        assert!(status.contains("core_dumps="));
        assert!(status.contains("panic_hook="));
    }

    #[test]
    fn strict_policy_fails_visibly_when_mlock_not_enforced() {
        // On typical CI hosts mlockall fails → Strict must return Err with report.
        match harden_agent_startup_with_policy(HardenPolicy::Strict) {
            Ok(report) => {
                // Privileged environment: all steps enforced.
                assert!(report.satisfies(HardenPolicy::Strict));
            }
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("not satisfied") || msg.contains("Strict"),
                    "unexpected error text: {msg}"
                );
                assert!(!e.report.satisfies(HardenPolicy::Strict));
                // Required floor still holds when Strict fails on mlock/dumpable alone.
                if e.report.core_dumps_disabled && e.report.panic_hook_installed {
                    assert!(e.report.satisfies(HardenPolicy::Required));
                }
            }
        }
    }
}
