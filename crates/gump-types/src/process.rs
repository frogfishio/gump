//! Best-effort process hardening before secret custody (SECURITY.md §8).
//!
//! Attempts core-dump disable, dumpability/ptrace restriction, memory locking,
//! and a redacting panic hook. Each step reports whether it was enforced — hosts
//! may lack privilege for `mlockall` / dumpable changes.
//!
//! Long-lived services (server / agent) use [`HardenPolicy::Required`] so startup
//! fails closed when core-dump disable or panic redaction cannot be applied
//! (STL-20).

use core::fmt;
use std::sync::Once;

static PANIC_HOOK: Once = Once::new();

/// How strictly [`prepare_for_custody_with_policy`] treats incomplete enforcement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HardenPolicy {
    /// Attempt all steps; always return the observational report (never Err).
    BestEffort,
    /// Fail unless core dumps are disabled and the redacting panic hook is installed.
    /// Dumpable/ptrace and `mlock` remain reported but optional (typical unprivileged hosts).
    Required,
    /// Fail unless every reported step succeeded (privileged / locked-down hosts).
    Strict,
}

/// Default policy for `gump-server` / `gump-agent` startup (STL-20).
pub const SERVICE_HARDEN_POLICY: HardenPolicy = HardenPolicy::Required;

/// Outcome of [`prepare_for_custody`] — every field is observational, not a
/// guarantee that an attacker with host-root cannot recover RAM.
///
/// `memory_locked` and `dumpable_or_attach_restricted` often stay `false` without
/// elevated privilege; callers must treat the report as telemetry, not a hard gate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessHardenReport {
    pub core_dumps_disabled: bool,
    pub dumpable_or_attach_restricted: bool,
    pub memory_locked: bool,
    pub panic_hook_installed: bool,
}

impl ProcessHardenReport {
    /// Whether this report meets `policy` (used for fail-closed service startup).
    pub fn satisfies(&self, policy: HardenPolicy) -> bool {
        match policy {
            HardenPolicy::BestEffort => true,
            HardenPolicy::Required => self.core_dumps_disabled && self.panic_hook_installed,
            HardenPolicy::Strict => {
                self.core_dumps_disabled
                    && self.dumpable_or_attach_restricted
                    && self.memory_locked
                    && self.panic_hook_installed
            }
        }
    }
}

impl fmt::Display for ProcessHardenReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "core_dumps={} dumpable/ptrace={} mlock={} panic_hook={}",
            self.core_dumps_disabled,
            self.dumpable_or_attach_restricted,
            self.memory_locked,
            self.panic_hook_installed
        )
    }
}

/// Fail-closed result when a [`HardenPolicy`] is not satisfied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HardenError {
    pub policy: HardenPolicy,
    pub report: ProcessHardenReport,
}

impl fmt::Display for HardenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "process harden policy {:?} not satisfied: {}",
            self.policy, self.report
        )
    }
}

impl std::error::Error for HardenError {}

/// Install redacting panic hook and attempt OS hardenings before custody material
/// enters process memory.
///
/// Equivalent to [`prepare_for_custody_with_policy`]`(HardenPolicy::BestEffort)` and
/// always succeeds (report may still show incomplete OS enforcement).
pub fn prepare_for_custody() -> ProcessHardenReport {
    match prepare_for_custody_with_policy(HardenPolicy::BestEffort) {
        Ok(r) => r,
        Err(_) => unreachable!("BestEffort never returns Err"),
    }
}

/// Service-oriented hardening with policy-controlled failure (STL-20 / SECURITY §8).
pub fn prepare_for_custody_with_policy(
    policy: HardenPolicy,
) -> Result<ProcessHardenReport, HardenError> {
    let report = attempt_harden();
    if report.satisfies(policy) {
        Ok(report)
    } else {
        Err(HardenError { policy, report })
    }
}

/// Harden using [`SERVICE_HARDEN_POLICY`], overridable via `GUMP_PROCESS_HARDEN`
/// (`best-effort` | `required` | `strict`).
pub fn prepare_service_for_custody() -> Result<ProcessHardenReport, HardenError> {
    prepare_for_custody_with_policy(service_harden_policy_from_env())
}

fn service_harden_policy_from_env() -> HardenPolicy {
    match std::env::var("GUMP_PROCESS_HARDEN").ok().as_deref() {
        Some("best-effort") | Some("best_effort") => HardenPolicy::BestEffort,
        Some("strict") => HardenPolicy::Strict,
        Some("required") | None => SERVICE_HARDEN_POLICY,
        Some(other) => {
            eprintln!(
                "gump: unknown GUMP_PROCESS_HARDEN={other:?}; using {:?}",
                SERVICE_HARDEN_POLICY
            );
            SERVICE_HARDEN_POLICY
        }
    }
}

fn attempt_harden() -> ProcessHardenReport {
    let panic_hook_installed = install_redacting_panic_hook();
    #[cfg(unix)]
    let (core_dumps_disabled, dumpable_or_attach_restricted, memory_locked) = sys::harden_unix();
    #[cfg(not(unix))]
    let (core_dumps_disabled, dumpable_or_attach_restricted, memory_locked) = (false, false, false);

    ProcessHardenReport {
        core_dumps_disabled,
        dumpable_or_attach_restricted,
        memory_locked,
        panic_hook_installed,
    }
}

fn install_redacting_panic_hook() -> bool {
    let mut installed_this_call = false;
    PANIC_HOOK.call_once(|| {
        std::panic::set_hook(Box::new(move |info| {
            // Never include panic payload Display — it may contain secret material.
            let loc = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                .unwrap_or_else(|| "<unknown>".into());
            eprintln!("gump: panic (payload redacted) at {loc}");
        }));
        installed_this_call = true;
    });
    // Once::call_once only sets the local flag on first call; later callers still
    // observe a hook installed by a prior prepare.
    installed_this_call || PANIC_HOOK.is_completed()
}

#[cfg(unix)]
mod sys {
    #![allow(unsafe_code)]

    use std::io;

    pub fn harden_unix() -> (bool, bool, bool) {
        let core = disable_core_dumps().is_ok();
        let attach = restrict_dumpable_or_attach().is_ok();
        let locked = lock_address_space().is_ok();
        (core, attach, locked)
    }

    fn disable_core_dumps() -> io::Result<()> {
        // SAFETY: setrlimit with a stack-local rlimit is well-defined.
        let lim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        let rc = unsafe { libc::setrlimit(libc::RLIMIT_CORE, &lim) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn restrict_dumpable_or_attach() -> io::Result<()> {
        // SAFETY: prctl PR_SET_DUMPABLE is a process-wide flag with no pointer args.
        let rc = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    fn restrict_dumpable_or_attach() -> io::Result<()> {
        // PT_DENY_ATTACH — request id 31 on Darwin / BSD.
        const PT_DENY_ATTACH: libc::c_int = 31;
        // SAFETY: ptrace(PT_DENY_ATTACH) takes null addr/data on Darwin.
        let rc = unsafe { libc::ptrace(PT_DENY_ATTACH, 0, std::ptr::null_mut(), 0) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    )))]
    fn restrict_dumpable_or_attach() -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "dumpable/ptrace harden unsupported on this unix",
        ))
    }

    fn lock_address_space() -> io::Result<()> {
        // SAFETY: mlockall flags are integers; failure is reported via errno.
        let rc = unsafe { libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_installs_panic_hook_and_reports() {
        let report = prepare_for_custody();
        assert!(report.panic_hook_installed);
        // Core dump disable should succeed for an ordinary user on unix.
        #[cfg(unix)]
        assert!(report.core_dumps_disabled);
    }

    #[test]
    fn required_policy_rejects_missing_core_or_panic_hook() {
        let missing_core = ProcessHardenReport {
            core_dumps_disabled: false,
            dumpable_or_attach_restricted: true,
            memory_locked: true,
            panic_hook_installed: true,
        };
        assert!(!missing_core.satisfies(HardenPolicy::Required));

        let missing_hook = ProcessHardenReport {
            core_dumps_disabled: true,
            dumpable_or_attach_restricted: true,
            memory_locked: true,
            panic_hook_installed: false,
        };
        assert!(!missing_hook.satisfies(HardenPolicy::Required));
    }

    #[test]
    fn strict_policy_fails_when_os_cannot_enforce_mlock() {
        // Fabricated report: core+panic ok, mlock not enforced — Strict must fail closed.
        let partial = ProcessHardenReport {
            core_dumps_disabled: true,
            dumpable_or_attach_restricted: true,
            memory_locked: false,
            panic_hook_installed: true,
        };
        assert!(partial.satisfies(HardenPolicy::Required));
        assert!(!partial.satisfies(HardenPolicy::Strict));
    }

    #[test]
    fn service_required_succeeds_on_unix_without_privilege() {
        let report = prepare_for_custody_with_policy(HardenPolicy::Required)
            .expect("Required must pass when core dumps + panic hook apply");
        assert!(report.panic_hook_installed);
        #[cfg(unix)]
        assert!(report.core_dumps_disabled);
    }
}
