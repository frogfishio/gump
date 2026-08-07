//! Best-effort process hardening before secret custody (SECURITY.md §8).
//!
//! Attempts core-dump disable, dumpability/ptrace restriction, memory locking,
//! and a redacting panic hook. Each step reports whether it was enforced — hosts
//! may lack privilege for `mlockall` / dumpable changes.

use core::fmt;
use std::sync::Once;

static PANIC_HOOK: Once = Once::new();

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

/// Install redacting panic hook and attempt OS hardenings before custody material
/// enters process memory.
pub fn prepare_for_custody() -> ProcessHardenReport {
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
}
