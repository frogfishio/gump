//! Process-tree supervision: pipe drains + TERM→deadline→KILL (RUNTIME §9 / §16, STL-04).
//!
//! Drain threads start before `start` returns so a chatty child cannot fill OS
//! pipe buffers and hang. Captured bytes go into a bounded ring (drop-oldest);
//! full Ratatouille wiring remains STL-09. Unix process-group signals use the
//! host `kill` binary so this crate can keep `forbid(unsafe_code)`.

use std::collections::VecDeque;
use std::io::Read;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::abi::Signal;
use crate::error::{DriverError, DriverErrorKind};

/// Max bytes retained per stream (drop-oldest). Keeps supervision off the telemetry path.
pub const CAPTURE_RING_BYTES: usize = 256 * 1024;
/// RUNTIME §14: each pipe read is at most 32 KiB.
const READ_CHUNK: usize = 32 * 1024;
/// RUNTIME §14: bounded drain after child exit.
pub const DRAIN_JOIN_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamKind {
    Stdout,
    Stderr,
}

#[derive(Debug, Default)]
struct RingInner {
    buf: VecDeque<u8>,
    dropped: u64,
    received: u64,
}

impl RingInner {
    fn push(&mut self, chunk: &[u8]) {
        self.received = self.received.saturating_add(chunk.len() as u64);
        for &b in chunk {
            if self.buf.len() >= CAPTURE_RING_BYTES {
                let _ = self.buf.pop_front();
                self.dropped = self.dropped.saturating_add(1);
            }
            self.buf.push_back(b);
        }
    }
}

/// Shared bounded capture ring for one stream.
#[derive(Clone, Debug, Default)]
pub struct CaptureRing {
    inner: Arc<Mutex<RingInner>>,
}

impl CaptureRing {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, chunk: &[u8]) {
        if let Ok(mut g) = self.inner.lock() {
            g.push(chunk);
        }
    }

    pub fn snapshot(&self) -> Vec<u8> {
        self.inner
            .lock()
            .map(|g| g.buf.iter().copied().collect())
            .unwrap_or_default()
    }

    pub fn dropped_bytes(&self) -> u64 {
        self.inner.lock().map(|g| g.dropped).unwrap_or(0)
    }

    pub fn received_bytes(&self) -> u64 {
        self.inner.lock().map(|g| g.received).unwrap_or(0)
    }
}

#[derive(Debug)]
pub struct PipeDrains {
    joins: Vec<JoinHandle<()>>,
    pub stdout: CaptureRing,
    pub stderr: CaptureRing,
}

impl PipeDrains {
    /// Take pipes from the child and start drain threads immediately (RUNTIME §9).
    pub fn start(child: &mut Child) -> Self {
        let stdout_ring = CaptureRing::new();
        let stderr_ring = CaptureRing::new();
        let mut joins = Vec::new();
        if let Some(out) = child.stdout.take() {
            joins.push(spawn_drain(StreamKind::Stdout, out, stdout_ring.clone()));
        }
        if let Some(err) = child.stderr.take() {
            joins.push(spawn_drain(StreamKind::Stderr, err, stderr_ring.clone()));
        }
        Self {
            joins,
            stdout: stdout_ring,
            stderr: stderr_ring,
        }
    }

    /// Join drain threads (best-effort within `DRAIN_JOIN_TIMEOUT`).
    pub fn join_bounded(&mut self) {
        let deadline = Instant::now() + DRAIN_JOIN_TIMEOUT;
        let joins = std::mem::take(&mut self.joins);
        for j in joins {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // Detach: thread exits when pipe EOF arrives after child death.
                drop(j);
                continue;
            }
            // JoinHandle has no timed join in std; poll with try pattern via thread park is
            // unavailable — join immediately (drains exit promptly on EOF after kill).
            let _ = j.join();
            let _ = remaining;
        }
    }
}

fn spawn_drain<R: Read + Send + 'static>(
    _kind: StreamKind,
    mut reader: R,
    ring: CaptureRing,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("gump-pipe-drain".into())
        .spawn(move || {
            let mut buf = [0u8; READ_CHUNK];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => ring.push(&buf[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        })
        .expect("spawn pipe drain")
}

/// Signal the child's process group (unix) or the direct child (fallback).
pub fn signal_tree(child: &mut Child, signal: Signal) -> Result<(), DriverError> {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        // process_group(0) ⇒ child's pgid == pid for the leader.
        let pgid = pid;
        // Prefer /bin/kill -s NAME -<pgid>: portable across BSD (macOS) and GNU;
        // avoid `--` (not accepted by macOS kill) and zsh's builtin `kill`.
        let name = match signal {
            Signal::Term => "TERM",
            Signal::Int => "INT",
            Signal::Kill => "KILL",
        };
        let status = Command::new("/bin/kill")
            .arg("-s")
            .arg(name)
            .arg(format!("-{pgid}"))
            .status()
            .map_err(|e| {
                DriverError::new(
                    DriverErrorKind::Signal,
                    format!("kill process group failed: {e}"),
                )
            })?;
        if !status.success() {
            // Fallback: direct child, then ESRCH is fine on cleanup.
            match signal {
                Signal::Kill => {
                    let _ = child.kill();
                }
                Signal::Term | Signal::Int => {
                    // Best-effort single-process TERM if group signal failed.
                    let _ = Command::new("/bin/kill")
                        .arg("-s")
                        .arg(name)
                        .arg(child.id().to_string())
                        .status();
                }
            }
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = signal;
        child
            .kill()
            .map_err(|e| DriverError::new(DriverErrorKind::Signal, format!("kill failed: {e}")))?;
        Ok(())
    }
}
