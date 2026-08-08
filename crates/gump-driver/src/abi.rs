//! `gump.driver/1` types and trait (RUNTIME.md §4).

use std::path::{Path, PathBuf};
use std::time::Duration;

use std::sync::Arc;

use gump_types::AttemptId;

use crate::error::DriverError;
use crate::supervisor::PipeChunkSink;

/// Semantic driver ABI version string.
pub const DRIVER_ABI: &str = "gump.driver/1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum DriverKind {
    Native,
    Script,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostProbe {
    pub os: String,
    pub arch: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DriverCapabilities {
    pub kind: DriverKind,
    pub abi: &'static str,
    pub supports_process_group: bool,
    pub supports_interpreter: bool,
}

/// Path to a verified, materialized release tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseRoot {
    pub path: PathBuf,
}

impl ReleaseRoot {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn as_path(&self) -> &Path {
        &self.path
    }
}

/// Runtime contract supplied to prepare/start (subset of normalized manifest).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeSpec {
    pub kind: DriverKind,
    /// Argument vector relative to the release root for native; for script,
    /// the script argv after the interpreter.
    pub command: Vec<String>,
    /// Explicit interpreter argv for script driver; must be absent for native.
    pub interpreter: Option<Vec<String>>,
    pub workdir: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptContext {
    pub attempt_id: AttemptId,
    /// Private directory owned by this attempt (created by prepare).
    pub attempt_root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceGrant {
    pub max_processes: Option<u32>,
}

pub use crate::secrets::SecretPlan;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartFence {
    pub generation: u64,
}

#[derive(Clone, Default)]
pub struct IoEndpoints {
    pub capture_stdout: bool,
    pub capture_stderr: bool,
    /// Optional fan-out into the bounded telemetry path (STL-09 / D011).
    pub pipe_sink: Option<Arc<dyn PipeChunkSink>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Signal {
    Term,
    Kill,
    Int,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Observation {
    pub running: bool,
    pub exit_code: Option<i32>,
}

/// Opaque prepared attempt (process-local; never serialized to K/V).
#[derive(Debug)]
pub struct PreparedHandle {
    pub(crate) attempt_id: AttemptId,
    pub(crate) attempt_root: PathBuf,
    pub(crate) release_root: PathBuf,
    pub(crate) argv: Vec<String>,
    pub(crate) workdir: PathBuf,
    pub(crate) admitted: bool,
}

/// Opaque running attempt.
#[derive(Debug)]
pub struct RunningHandle {
    #[allow(dead_code)] // attempt/release roots retained for R06 supervision
    pub(crate) prepared: PreparedHandle,
    pub(crate) child: Option<std::process::Child>,
    pub(crate) drains: Option<crate::supervisor::PipeDrains>,
    #[allow(dead_code)] // retained for fence-aware supervision in R06
    pub(crate) fence: StartFence,
    /// Primary exit status preserved across terminal finalization (STL-22).
    pub(crate) last_exit_code: Option<i32>,
}

impl RunningHandle {
    pub fn attempt_id(&self) -> AttemptId {
        self.prepared.attempt_id
    }

    pub fn attempt_root(&self) -> &std::path::Path {
        &self.prepared.attempt_root
    }

    pub fn fence_generation(&self) -> u64 {
        self.fence.generation
    }

    /// Finalize the process tree and return a handle for [`Driver::cleanup`].
    ///
    /// Used by the agent supervision loop (GUMP-N012 / R06) after observe/kill.
    pub fn into_prepared(mut self) -> PreparedHandle {
        self.finalize_terminal(None);
        PreparedHandle {
            attempt_id: self.prepared.attempt_id,
            attempt_root: std::mem::take(&mut self.prepared.attempt_root),
            release_root: std::mem::take(&mut self.prepared.release_root),
            argv: std::mem::take(&mut self.prepared.argv),
            workdir: std::mem::take(&mut self.prepared.workdir),
            admitted: self.prepared.admitted,
        }
    }

    /// Bytes retained from stdout (bounded ring; may have dropped oldest).
    pub fn captured_stdout(&self) -> Vec<u8> {
        self.drains
            .as_ref()
            .map(|d| d.stdout.snapshot())
            .unwrap_or_default()
    }

    /// Bytes retained from stderr (bounded ring; may have dropped oldest).
    pub fn captured_stderr(&self) -> Vec<u8> {
        self.drains
            .as_ref()
            .map(|d| d.stderr.snapshot())
            .unwrap_or_default()
    }

    /// Total stdout bytes received (including dropped).
    pub fn stdout_received_bytes(&self) -> u64 {
        self.drains
            .as_ref()
            .map(|d| d.stdout.received_bytes())
            .unwrap_or(0)
    }

    /// Total stderr bytes received (including dropped).
    pub fn stderr_received_bytes(&self) -> u64 {
        self.drains
            .as_ref()
            .map(|d| d.stderr.received_bytes())
            .unwrap_or(0)
    }

    /// STL-22: kill/reap the process group (or cgroup fallback), finish bounded
    /// pipe drains, and clear the running handle.
    ///
    /// Call after the primary has exited (or when forcing kill). When
    /// `primary_exit` is `Some`, it is recorded as the authoritative exit code
    /// before reaping; otherwise the code from `Child::wait` is kept if present.
    pub(crate) fn finalize_terminal(&mut self, primary_exit: Option<Option<i32>>) {
        if let Some(code) = primary_exit {
            self.last_exit_code = code;
        }
        if self.child.is_none() {
            // Child already cleared; still finish any outstanding drain joins.
            if let Some(drains) = self.drains.as_mut() {
                drains.join_bounded();
            }
            return;
        }
        if let Some(child) = self.child.as_mut() {
            // Remaining descendants in the process group must not outlive observation.
            let _ = crate::supervisor::signal_tree(child, Signal::Kill);
        }
        if let Some(mut child) = self.child.take() {
            if let Ok(status) = child.wait() {
                if primary_exit.is_none() && self.last_exit_code.is_none() {
                    self.last_exit_code = status.code();
                }
            }
        }
        // Join drain threads but keep CaptureRings for post-mortem inspection.
        if let Some(drains) = self.drains.as_mut() {
            drains.join_bounded();
        }
    }
}

impl Drop for RunningHandle {
    fn drop(&mut self) {
        // STL-22: emergency containment if the caller dropped without observe/kill.
        self.finalize_terminal(None);
    }
}

/// Admission token proving local feasibility without starting work.
#[derive(Debug)]
pub struct Admission {
    pub(crate) prepared: PreparedHandle,
    pub(crate) grant: ResourceGrant,
    pub(crate) secrets: SecretPlan,
}

/// Driver lifecycle trait (`gump.driver/1`).
pub trait Driver {
    fn probe(&self, host: &HostProbe) -> Result<DriverCapabilities, DriverError>;

    fn prepare(
        &self,
        release: &ReleaseRoot,
        runtime: &RuntimeSpec,
        ctx: &AttemptContext,
    ) -> Result<PreparedHandle, DriverError>;

    fn admit(
        &self,
        prepared: PreparedHandle,
        grant: ResourceGrant,
        secrets: SecretPlan,
    ) -> Result<Admission, DriverError>;

    fn start(
        &self,
        admission: Admission,
        fence: StartFence,
        io: &IoEndpoints,
    ) -> Result<RunningHandle, DriverError>;

    fn observe(&self, running: &mut RunningHandle) -> Result<Observation, DriverError>;

    fn signal(&self, running: &mut RunningHandle, signal: Signal) -> Result<(), DriverError>;

    fn terminate(&self, running: &mut RunningHandle, deadline: Duration)
    -> Result<(), DriverError>;

    fn kill(&self, running: &mut RunningHandle) -> Result<(), DriverError>;

    fn cleanup(&self, prepared: PreparedHandle) -> Result<(), DriverError>;
}
