//! Fenced effect executor / supervision loop (GUMP-N012 / R06 / R09 / R10).
//!
//! Reconcile accepted placements → materialize verified Capsules only →
//! prepare/admit/start via driver ABI → observe → cleanup attempt roots.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gump_driver::{
    AttemptContext, Driver, DriverError, IoEndpoints, Observation, ReleaseRoot, ResourceGrant,
    RunningHandle, RuntimeSpec, SecretPlan, StartFence,
};
use gump_types::{AttemptId, UnitId};

use crate::fence::{
    AuthorityState, EffectKind, FenceError, IsolationPolicy, allow_effect, isolation_grace_expired,
    require_fence,
};

/// Ceiling on concurrent live attempts tracked by one agent (bounded).
pub const DEFAULT_MAX_LIVE_ATTEMPTS: usize = 4_096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentError {
    Fence(FenceError),
    Driver(String),
    UnverifiedCapsule,
    AttemptRoot(String),
    Capacity,
    NotFound,
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fence(e) => write!(f, "{e}"),
            Self::Driver(e) => write!(f, "driver: {e}"),
            Self::UnverifiedCapsule => {
                write!(f, "refusing to materialize unverified Capsule")
            }
            Self::AttemptRoot(e) => write!(f, "attempt root: {e}"),
            Self::Capacity => write!(f, "live attempt capacity reached"),
            Self::NotFound => write!(f, "attempt not found"),
        }
    }
}

impl std::error::Error for AgentError {}

impl From<FenceError> for AgentError {
    fn from(e: FenceError) -> Self {
        Self::Fence(e)
    }
}

impl From<DriverError> for AgentError {
    fn from(e: DriverError) -> Self {
        Self::Driver(e.to_string())
    }
}

/// Desired unit accepted by the controller for this agent node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedPlacement {
    pub attempt_id: AttemptId,
    pub unit_id: UnitId,
    pub placement_fence: u64,
    pub release_root: PathBuf,
    pub runtime: RuntimeSpec,
    pub lifecycle_finite: bool,
    /// Capsule bytes must be fully verified before materialize/start (fail closed).
    pub capsule_verified: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttemptPhase {
    Starting,
    Running,
    Terminal { exit_code: Option<i32> },
    Stopped,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptReport {
    pub attempt_id: AttemptId,
    pub unit_id: UnitId,
    pub phase: AttemptPhase,
    pub placement_fence: u64,
    pub observation: Option<Observation>,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
}

struct LiveAttempt {
    unit_id: UnitId,
    placement_fence: u64,
    lifecycle_finite: bool,
    attempt_root: PathBuf,
    running: Option<RunningHandle>,
    phase: AttemptPhase,
    last_obs: Option<Observation>,
}

/// Agent-local reconciler: owns attempt roots and driver effects under a fence.
pub struct EffectExecutor<D: Driver> {
    driver: D,
    attempts_base: PathBuf,
    authority: AuthorityState,
    isolation: IsolationPolicy,
    live: BTreeMap<AttemptId, LiveAttempt>,
    max_live: usize,
}

impl<D: Driver> EffectExecutor<D> {
    pub fn new(driver: D, attempts_base: PathBuf, authority: AuthorityState) -> Self {
        Self {
            driver,
            attempts_base,
            authority,
            isolation: IsolationPolicy::default(),
            live: BTreeMap::new(),
            max_live: DEFAULT_MAX_LIVE_ATTEMPTS,
        }
    }

    pub fn with_isolation(mut self, policy: IsolationPolicy) -> Self {
        self.isolation = policy;
        self
    }

    pub fn authority(&self) -> &AuthorityState {
        &self.authority
    }

    pub fn set_authority(&mut self, authority: AuthorityState) {
        self.authority = authority;
    }

    pub fn mark_isolated(&mut self, now_ms: u64) {
        if self.authority.isolated_since_ms.is_none() {
            self.authority.isolated_since_ms = Some(now_ms);
        }
    }

    pub fn clear_isolation(&mut self, placement_fence: u64, controller_epoch: u64) {
        self.authority.isolated_since_ms = None;
        self.authority.placement_fence = placement_fence;
        self.authority.controller_epoch = controller_epoch;
    }

    pub fn live_count(&self) -> usize {
        self.live.len()
    }

    pub fn report(&self, id: AttemptId) -> Result<AttemptReport, AgentError> {
        allow_effect(
            &self.authority,
            self.authority.placement_fence,
            EffectKind::Report,
        )?;
        let a = self.live.get(&id).ok_or(AgentError::NotFound)?;
        // Per-attempt fence must still match live authority for report.
        require_fence(&self.authority, a.placement_fence)?;
        Ok(self.make_report(id, a))
    }

    fn make_report(&self, id: AttemptId, a: &LiveAttempt) -> AttemptReport {
        let (stdout_bytes, stderr_bytes) = match &a.running {
            Some(r) => (r.captured_stdout().len(), r.captured_stderr().len()),
            None => (0, 0),
        };
        AttemptReport {
            attempt_id: id,
            unit_id: a.unit_id,
            phase: a.phase.clone(),
            placement_fence: a.placement_fence,
            observation: a.last_obs.clone(),
            stdout_bytes,
            stderr_bytes,
        }
    }

    /// One reconcile pass against the desired placement set at `now_ms`.
    pub fn reconcile(
        &mut self,
        desired: &[AcceptedPlacement],
        now_ms: u64,
    ) -> Result<Vec<AttemptReport>, AgentError> {
        // Grace expiry forces stop+cleanup of everything still running.
        if isolation_grace_expired(&self.authority, &self.isolation, now_ms) {
            let ids: Vec<AttemptId> = self.live.keys().copied().collect();
            for id in ids {
                let _ = self.force_stop_cleanup(id);
            }
            return Err(AgentError::Fence(FenceError::GraceExpired));
        }

        // While isolated: continue polling OS state only — no start/stop/report
        // effects (RUNTIME.md §10 / INV-014).
        if self.authority.is_isolated() {
            let ids: Vec<AttemptId> = self.live.keys().copied().collect();
            for id in ids {
                self.observe_one(id)?;
            }
            return Ok(Vec::new());
        }

        let desired_ids: BTreeMap<AttemptId, &AcceptedPlacement> =
            desired.iter().map(|p| (p.attempt_id, p)).collect();

        // Remove units no longer desired (intent change).
        let obsolete: Vec<AttemptId> = self
            .live
            .keys()
            .copied()
            .filter(|id| !desired_ids.contains_key(id))
            .collect();
        for id in obsolete {
            self.stop_and_cleanup(id)?;
        }

        // Start missing desired units (finite or continuous).
        for p in desired {
            if self.live.contains_key(&p.attempt_id) {
                continue;
            }
            self.start_placement(p)?;
        }

        // Observe running attempts; finite terminals clean up.
        let ids: Vec<AttemptId> = self.live.keys().copied().collect();
        for id in ids {
            self.observe_one(id)?;
        }

        Ok(self
            .live
            .iter()
            .map(|(id, a)| self.make_report(*id, a))
            .collect())
    }

    fn start_placement(&mut self, p: &AcceptedPlacement) -> Result<(), AgentError> {
        allow_effect(&self.authority, p.placement_fence, EffectKind::Start)?;
        if !p.capsule_verified {
            return Err(AgentError::UnverifiedCapsule);
        }
        if self.live.len() >= self.max_live {
            return Err(AgentError::Capacity);
        }
        if !p.release_root.is_dir() {
            return Err(AgentError::AttemptRoot(format!(
                "release root missing: {}",
                p.release_root.display()
            )));
        }

        let attempt_root = self.attempts_base.join(p.attempt_id.to_hyphenated());
        if attempt_root.exists() {
            fs::remove_dir_all(&attempt_root)
                .map_err(|e| AgentError::AttemptRoot(format!("clear attempt root: {e}")))?;
        }
        fs::create_dir_all(&attempt_root)
            .map_err(|e| AgentError::AttemptRoot(format!("create attempt root: {e}")))?;

        let release = ReleaseRoot {
            path: p.release_root.clone(),
        };
        let ctx = AttemptContext {
            attempt_id: p.attempt_id,
            attempt_root: attempt_root.clone(),
        };
        let prepared = self.driver.prepare(&release, &p.runtime, &ctx)?;
        let admission = self.driver.admit(
            prepared,
            ResourceGrant {
                max_processes: Some(64),
            },
            SecretPlan::deferred(),
        )?;
        let running = self.driver.start(
            admission,
            StartFence {
                generation: p.placement_fence,
            },
            &IoEndpoints {
                capture_stdout: true,
                capture_stderr: true,
                pipe_sink: None,
            },
        )?;

        self.live.insert(
            p.attempt_id,
            LiveAttempt {
                unit_id: p.unit_id,
                placement_fence: p.placement_fence,
                lifecycle_finite: p.lifecycle_finite,
                attempt_root,
                running: Some(running),
                phase: AttemptPhase::Running,
                last_obs: None,
            },
        );
        Ok(())
    }

    fn observe_one(&mut self, id: AttemptId) -> Result<(), AgentError> {
        let fence = self
            .live
            .get(&id)
            .map(|a| a.placement_fence)
            .ok_or(AgentError::NotFound)?;
        // Local OS poll is always allowed for already-running attempts; emitting
        // a report to the controller is gated separately via [`Self::report`].
        require_fence(&self.authority, fence)?;

        let finite = self.live.get(&id).map(|a| a.lifecycle_finite).unwrap();
        let obs = {
            let a = self.live.get_mut(&id).ok_or(AgentError::NotFound)?;
            match a.running.as_mut() {
                Some(r) => self.driver.observe(r)?,
                None => Observation {
                    running: false,
                    exit_code: match a.phase {
                        AttemptPhase::Terminal { exit_code } => exit_code,
                        _ => None,
                    },
                },
            }
        };

        if obs.running {
            if let Some(a) = self.live.get_mut(&id) {
                a.phase = AttemptPhase::Running;
                a.last_obs = Some(obs);
            }
            return Ok(());
        }

        // Terminal.
        if let Some(a) = self.live.get_mut(&id) {
            a.phase = AttemptPhase::Terminal {
                exit_code: obs.exit_code,
            };
            a.last_obs = Some(obs);
        }
        // Finite completions clean up immediately; continuous waits for intent change.
        // Do not cleanup while isolated — that is a stop effect (INV-014).
        if finite && !self.authority.is_isolated() {
            self.cleanup_live(id)?;
        }
        Ok(())
    }

    pub fn cancel(&mut self, id: AttemptId) -> Result<(), AgentError> {
        let fence = self
            .live
            .get(&id)
            .map(|a| a.placement_fence)
            .ok_or(AgentError::NotFound)?;
        allow_effect(&self.authority, fence, EffectKind::Stop)?;
        self.stop_and_cleanup(id)
    }

    pub fn force_kill(&mut self, id: AttemptId) -> Result<(), AgentError> {
        let fence = self
            .live
            .get(&id)
            .map(|a| a.placement_fence)
            .ok_or(AgentError::NotFound)?;
        allow_effect(&self.authority, fence, EffectKind::Stop)?;
        if let Some(a) = self.live.get_mut(&id) {
            if let Some(r) = a.running.as_mut() {
                self.driver.kill(r)?;
            }
        }
        self.cleanup_live(id)
    }

    fn stop_and_cleanup(&mut self, id: AttemptId) -> Result<(), AgentError> {
        let fence = self
            .live
            .get(&id)
            .map(|a| a.placement_fence)
            .ok_or(AgentError::NotFound)?;
        allow_effect(&self.authority, fence, EffectKind::Stop)?;
        if let Some(a) = self.live.get_mut(&id) {
            if let Some(r) = a.running.as_mut() {
                let _ = self.driver.terminate(r, Duration::from_millis(200));
                let obs = self.driver.observe(r)?;
                if obs.running {
                    self.driver.kill(r)?;
                }
            }
            a.phase = AttemptPhase::Stopped;
        }
        self.cleanup_live(id)
    }

    /// Isolation grace path: stop without allow_effect (authority already expired).
    fn force_stop_cleanup(&mut self, id: AttemptId) -> Result<(), AgentError> {
        if let Some(a) = self.live.get_mut(&id) {
            if let Some(r) = a.running.as_mut() {
                let _ = self.driver.kill(r);
            }
        }
        self.cleanup_live(id)
    }

    fn cleanup_live(&mut self, id: AttemptId) -> Result<(), AgentError> {
        let Some(mut a) = self.live.remove(&id) else {
            return Ok(());
        };
        if let Some(running) = a.running.take() {
            let prepared = running.into_prepared();
            self.driver.cleanup(prepared)?;
        } else if a.attempt_root.exists() {
            // No running handle — still remove the owned root.
            remove_attempt_root(&a.attempt_root)?;
        }
        Ok(())
    }

    /// Test/ops: whether the attempt root still exists on disk.
    pub fn attempt_root_exists(&self, id: AttemptId) -> bool {
        self.live
            .get(&id)
            .map(|a| a.attempt_root.exists())
            .unwrap_or_else(|| self.attempts_base.join(id.to_hyphenated()).exists())
    }
}

fn remove_attempt_root(path: &Path) -> Result<(), AgentError> {
    if path.exists() {
        fs::remove_dir_all(path)
            .map_err(|e| AgentError::AttemptRoot(format!("cleanup {}: {e}", path.display())))?;
    }
    Ok(())
}
