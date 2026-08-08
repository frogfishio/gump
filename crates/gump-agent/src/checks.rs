//! Non-blocking-enough health/completion probes (RUNTIME.md §11).
//!
//! Each reconcile tick is given a wall-clock budget so checks cannot stall
//! the control plane indefinitely.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::lifecycle::{CheckKind, CheckSpec, reasons};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckOutcome {
    pub ok: bool,
    pub reason_code: &'static str,
    pub detail: String,
    pub elapsed_ms: u64,
}

/// Remaining wall-clock budget for checks in one reconcile pass.
#[derive(Clone, Copy, Debug)]
pub struct CheckBudget {
    deadline: Instant,
}

impl CheckBudget {
    pub fn new(max_ms: u64) -> Self {
        Self {
            deadline: Instant::now() + Duration::from_millis(max_ms.max(1)),
        }
    }

    pub fn exhausted(&self) -> bool {
        Instant::now() >= self.deadline
    }

    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }
}

/// Run one check, respecting `spec.timeout_ms` and the shared reconcile budget.
pub fn run_check(spec: &CheckSpec, process_running: bool, budget: &CheckBudget) -> CheckOutcome {
    if budget.exhausted() {
        return CheckOutcome {
            ok: false,
            reason_code: reasons::CHECK_SKIPPED_BUDGET,
            detail: "check skipped: reconcile budget exhausted".into(),
            elapsed_ms: 0,
        };
    }
    let timeout = Duration::from_millis(spec.timeout_ms.max(1)).min(budget.remaining());
    let start = Instant::now();
    let result = match spec.kind {
        CheckKind::Process => Ok(process_running),
        CheckKind::Tcp => probe_tcp(spec.target.as_deref(), timeout),
        CheckKind::Http => probe_http(spec.target.as_deref(), timeout),
        CheckKind::Command => probe_command(spec, timeout),
    };
    let elapsed_ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(true) => CheckOutcome {
            ok: true,
            reason_code: "lifecycle.check_ok",
            detail: "check succeeded".into(),
            elapsed_ms,
        },
        Ok(false) => CheckOutcome {
            ok: false,
            reason_code: reasons::READINESS_FAILED,
            detail: "check failed".into(),
            elapsed_ms,
        },
        Err(detail) => CheckOutcome {
            ok: false,
            reason_code: reasons::CHECK_TIMEOUT,
            detail,
            elapsed_ms,
        },
    }
}

fn probe_tcp(target: Option<&str>, timeout: Duration) -> Result<bool, String> {
    let target = target.ok_or_else(|| "tcp check missing target".to_string())?;
    let addr = target
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or_else(|| "tcp target unresolved".to_string())?;
    match TcpStream::connect_timeout(&addr, timeout) {
        Ok(_) => Ok(true),
        Err(e) => Err(e.to_string()),
    }
}

fn probe_http(target: Option<&str>, timeout: Duration) -> Result<bool, String> {
    let raw = target.ok_or_else(|| "http check missing target".to_string())?;
    let url = raw.strip_prefix("http://").unwrap_or(raw);
    let (hostport, path) = match url.split_once('/') {
        Some((hp, rest)) => (hp, format!("/{rest}")),
        None => (url, "/".to_string()),
    };
    let addr = hostport
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or_else(|| "http target unresolved".to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    let host = hostport.split(':').next().unwrap_or(hostport);
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nHiccup-Offer: 1\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut buf = [0u8; 128];
    let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
    let head = std::str::from_utf8(&buf[..n]).unwrap_or("");
    // HTTP/1.x 2xx / 3xx
    let ok = head.contains(" 200 ")
        || head.contains(" 201 ")
        || head.contains(" 204 ")
        || head.contains(" 301 ")
        || head.contains(" 302 ")
        || head.contains(" 307 ")
        || head.contains(" 308 ");
    Ok(ok)
}

fn probe_command(spec: &CheckSpec, timeout: Duration) -> Result<bool, String> {
    let argv = spec
        .command
        .as_ref()
        .ok_or_else(|| "command check missing argv".to_string())?;
    if argv.is_empty() {
        return Err("command argv empty".into());
    }
    let mut cmd = Command::new(&argv[0]);
    if argv.len() > 1 {
        cmd.args(&argv[1..]);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear();
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let start = Instant::now();
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => {
                // Drain bounded output (discard; never log secrets).
                if let Some(mut out) = child.stdout.take() {
                    let mut buf = Vec::new();
                    let _ = out
                        .by_ref()
                        .take(spec.max_output_bytes as u64)
                        .read_to_end(&mut buf);
                }
                return Ok(status.success());
            }
            None if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("command check timed out".into());
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::CheckKind;

    #[test]
    fn budget_exhaustion_skips_without_blocking() {
        let budget = CheckBudget {
            deadline: Instant::now() - Duration::from_millis(1),
        };
        let spec = CheckSpec::process_default();
        let out = run_check(&spec, true, &budget);
        assert!(!out.ok);
        assert_eq!(out.reason_code, reasons::CHECK_SKIPPED_BUDGET);
    }

    #[test]
    fn process_check_reflects_running_flag() {
        let budget = CheckBudget::new(100);
        let spec = CheckSpec {
            kind: CheckKind::Process,
            ..CheckSpec::process_default()
        };
        assert!(run_check(&spec, true, &budget).ok);
        assert!(!run_check(&spec, false, &budget).ok);
    }
}
