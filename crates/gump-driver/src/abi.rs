//! `gump.driver/1` types and trait (RUNTIME.md §4).

use std::path::{Path, PathBuf};
use std::time::Duration;

use gump_types::AttemptId;

use crate::error::DriverError;

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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SecretPlan {
    /// F06 contract suite: secrets are not delivered yet (R06/S07).
    pub deferred: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartFence {
    pub generation: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IoEndpoints {
    pub capture_stdout: bool,
    pub capture_stderr: bool,
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
}

impl RunningHandle {
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
}

/// Admission token proving local feasibility without starting work.
#[derive(Debug)]
pub struct Admission {
    pub(crate) prepared: PreparedHandle,
    pub(crate) grant: ResourceGrant,
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
        secrets: &SecretPlan,
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
