//! Fenced effect executor / supervision loop (GUMP-N012–N013 / R06 / R09 / R10).
//!
//! Reconcile accepted placements → materialize verified Capsules only →
//! prepare/admit/start via driver ABI → checks/retry → observe → cleanup.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gump_driver::{
    AttemptContext, Driver, DriverError, IoEndpoints, Observation, ReleaseRoot, ResourceGrant,
    RunningHandle, RuntimeSpec, SecretPlan, StartFence,
};
use gump_types::{AttemptId, UnitId};

use crate::checks::{CheckBudget, HttpRequestPlan, http_exchange, run_check};
use crate::fence::{
    AuthorityState, EffectKind, FenceError, IsolationPolicy, allow_effect, isolation_grace_expired,
    require_fence,
};
use crate::hiccup_bridge::{HealthOkCtx, HiccupPlacement, HiccupPlane};
use crate::lifecycle::{CheckKind, CheckRuntime, LifecycleContract, TerminalReason, reasons};
use gump_hiccup::OutboundHealth;

/// Ceiling on concurrent live attempts tracked by one agent (bounded).
pub const DEFAULT_MAX_LIVE_ATTEMPTS: usize = 4_096;
/// Per-reconcile wall-clock budget for health checks (must not block indefinitely).
pub const DEFAULT_CHECK_BUDGET_MS: u64 = 50;

pub type SecretPlanProvider =
    Arc<dyn Fn(&AcceptedPlacement) -> Result<SecretPlan, String> + Send + Sync>;
pub type PipeSinkFactory =
    Arc<dyn Fn(AttemptId) -> Arc<dyn gump_driver::PipeChunkSink> + Send + Sync>;

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
    /// Optional checks / retry (GUMP-N013). Default = no health, no retry.
    pub lifecycle: LifecycleContract,
    /// When set, successful HTTP health participates in one-node Hiccup (GUMP-N017).
    pub hiccup: Option<HiccupPlacement>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttemptPhase {
    Starting,
    Running,
    Terminal { exit_code: Option<i32> },
    AwaitingRestart { after_ms: u64 },
    PermanentFailure,
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
    /// `None` when readiness is undeclared (never inferred).
    pub ready: Option<bool>,
    /// `None` when publication is undeclared (never inferred).
    pub publication_eligible: Option<bool>,
    pub attempt_index: u32,
    pub terminal_reason: Option<TerminalReason>,
}

struct LiveAttempt {
    unit_id: UnitId,
    placement_fence: u64,
    lifecycle_finite: bool,
    lifecycle: LifecycleContract,
    release_root: PathBuf,
    runtime: RuntimeSpec,
    capsule_verified: bool,
    attempt_root: PathBuf,
    running: Option<RunningHandle>,
    phase: AttemptPhase,
    last_obs: Option<Observation>,
    started_ms: u64,
    /// 1-based execution index (increments on each restart).
    attempt_index: u32,
    readiness: CheckRuntime,
    liveness: CheckRuntime,
    ready: Option<bool>,
    publication_eligible: Option<bool>,
    terminal_reason: Option<TerminalReason>,
    hiccup: Option<HiccupPlacement>,
}

/// Agent-local reconciler: owns attempt roots and driver effects under a fence.
pub struct EffectExecutor<D: Driver> {
    driver: D,
    attempts_base: PathBuf,
    authority: AuthorityState,
    isolation: IsolationPolicy,
    live: BTreeMap<AttemptId, LiveAttempt>,
    /// Successful finite attempts remain converged while their placement is
    /// still desired. Without this tombstone, cleanup followed by the next
    /// reconcile pass would launch the same finite execution again.
    completed: BTreeSet<AttemptId>,
    completion_events: BTreeSet<AttemptId>,
    max_live: usize,
    check_budget_ms: u64,
    hiccup: HiccupPlane,
    secret_provider: Option<SecretPlanProvider>,
    pipe_sink_factory: Option<PipeSinkFactory>,
}

impl<D: Driver> EffectExecutor<D> {
    pub fn new(driver: D, attempts_base: PathBuf, authority: AuthorityState) -> Self {
        Self {
            driver,
            attempts_base,
            authority,
            isolation: IsolationPolicy::default(),
            live: BTreeMap::new(),
            completed: BTreeSet::new(),
            completion_events: BTreeSet::new(),
            max_live: DEFAULT_MAX_LIVE_ATTEMPTS,
            check_budget_ms: DEFAULT_CHECK_BUDGET_MS,
            hiccup: HiccupPlane::new(),
            secret_provider: None,
            pipe_sink_factory: None,
        }
    }

    /// Shared one-node Hiccup presence board (GUMP-N017).
    pub fn hiccup_plane(&self) -> &HiccupPlane {
        &self.hiccup
    }

    pub fn with_isolation(mut self, policy: IsolationPolicy) -> Self {
        self.isolation = policy;
        self
    }

    pub fn with_check_budget_ms(mut self, ms: u64) -> Self {
        self.check_budget_ms = ms.max(1);
        self
    }

    /// Install the custody-backed provider used immediately before admission.
    /// The default remains a plaintext-free deferred plan for workloads with no
    /// declared runtime values.
    pub fn with_secret_provider(mut self, provider: SecretPlanProvider) -> Self {
        self.secret_provider = Some(provider);
        self
    }

    /// Install a per-attempt bounded Ratatouille bridge factory.
    pub fn with_pipe_sink_factory(mut self, factory: PipeSinkFactory) -> Self {
        self.pipe_sink_factory = Some(factory);
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

    pub fn completed_count(&self) -> usize {
        self.completed.len()
    }

    /// Pending finite-success events awaiting authoritative cluster recording.
    pub fn completion_events(&self) -> Vec<AttemptId> {
        self.completion_events.iter().copied().collect()
    }

    pub fn acknowledge_completion(&mut self, attempt_id: AttemptId) {
        self.completion_events.remove(&attempt_id);
    }

    pub fn report(&self, id: AttemptId) -> Result<AttemptReport, AgentError> {
        allow_effect(
            &self.authority,
            self.authority.placement_fence,
            EffectKind::Report,
        )?;
        let a = self.live.get(&id).ok_or(AgentError::NotFound)?;
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
            ready: a.ready,
            publication_eligible: a.publication_eligible,
            attempt_index: a.attempt_index,
            terminal_reason: a.terminal_reason.clone(),
        }
    }

    /// One reconcile pass against the desired placement set at `now_ms`.
    pub fn reconcile(
        &mut self,
        desired: &[AcceptedPlacement],
        now_ms: u64,
    ) -> Result<Vec<AttemptReport>, AgentError> {
        if isolation_grace_expired(&self.authority, &self.isolation, now_ms) {
            let ids: Vec<AttemptId> = self.live.keys().copied().collect();
            for id in ids {
                let _ = self.force_stop_cleanup(id);
            }
            return Err(AgentError::Fence(FenceError::GraceExpired));
        }

        if self.authority.is_isolated() {
            let ids: Vec<AttemptId> = self.live.keys().copied().collect();
            for id in ids {
                self.observe_one(id, now_ms)?;
            }
            return Ok(Vec::new());
        }

        let desired_ids: BTreeMap<AttemptId, &AcceptedPlacement> =
            desired.iter().map(|p| (p.attempt_id, p)).collect();
        self.completed.retain(|id| desired_ids.contains_key(id));
        self.completion_events
            .retain(|id| desired_ids.contains_key(id));

        let obsolete: Vec<AttemptId> = self
            .live
            .keys()
            .copied()
            .filter(|id| !desired_ids.contains_key(id))
            .collect();
        for id in obsolete {
            self.stop_and_cleanup(id)?;
        }

        for p in desired {
            if self.live.contains_key(&p.attempt_id) || self.completed.contains(&p.attempt_id) {
                continue;
            }
            self.start_placement(p, now_ms, 1)?;
        }

        let mut budget = CheckBudget::new(self.check_budget_ms);
        let ids: Vec<AttemptId> = self.live.keys().copied().collect();
        for id in ids {
            self.maybe_restart(id, now_ms)?;
            self.observe_one(id, now_ms)?;
            self.tick_checks(id, now_ms, &mut budget)?;
        }

        Ok(self
            .live
            .iter()
            .map(|(id, a)| self.make_report(*id, a))
            .collect())
    }

    fn start_placement(
        &mut self,
        p: &AcceptedPlacement,
        now_ms: u64,
        attempt_index: u32,
    ) -> Result<(), AgentError> {
        let effect = if attempt_index > 1 {
            EffectKind::Restart
        } else {
            EffectKind::Start
        };
        allow_effect(&self.authority, p.placement_fence, effect)?;
        if !p.capsule_verified {
            return Err(AgentError::UnverifiedCapsule);
        }
        if attempt_index == 1 && self.live.len() >= self.max_live {
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
        let secrets = match &self.secret_provider {
            Some(provider) => provider(p).map_err(AgentError::Driver)?,
            None => SecretPlan::deferred(),
        };
        let admission = self.driver.admit(
            prepared,
            ResourceGrant {
                max_processes: Some(64),
            },
            secrets,
        )?;
        let running = self.driver.start(
            admission,
            StartFence {
                generation: p.placement_fence,
            },
            &IoEndpoints {
                capture_stdout: true,
                capture_stderr: true,
                pipe_sink: self
                    .pipe_sink_factory
                    .as_ref()
                    .map(|factory| factory(p.attempt_id)),
            },
        )?;

        let ready = p.lifecycle.readiness.as_ref().map(|_| false);
        let publication_eligible = if p.lifecycle.declares_publication {
            Some(false)
        } else {
            None
        };

        self.live.insert(
            p.attempt_id,
            LiveAttempt {
                unit_id: p.unit_id,
                placement_fence: p.placement_fence,
                lifecycle_finite: p.lifecycle_finite,
                lifecycle: p.lifecycle.clone(),
                release_root: p.release_root.clone(),
                runtime: p.runtime.clone(),
                capsule_verified: p.capsule_verified,
                attempt_root,
                running: Some(running),
                phase: AttemptPhase::Running,
                last_obs: None,
                started_ms: now_ms,
                attempt_index,
                readiness: CheckRuntime::default(),
                liveness: CheckRuntime::default(),
                ready,
                publication_eligible,
                terminal_reason: None,
                hiccup: p.hiccup.clone(),
            },
        );
        Ok(())
    }

    fn observe_one(&mut self, id: AttemptId, now_ms: u64) -> Result<(), AgentError> {
        let fence = self
            .live
            .get(&id)
            .map(|a| a.placement_fence)
            .ok_or(AgentError::NotFound)?;
        require_fence(&self.authority, fence)?;

        if matches!(
            self.live.get(&id).map(|a| &a.phase),
            Some(AttemptPhase::AwaitingRestart { .. } | AttemptPhase::PermanentFailure)
        ) {
            return Ok(());
        }

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

        self.apply_exit_policy(id, obs, now_ms)
    }

    fn apply_exit_policy(
        &mut self,
        id: AttemptId,
        obs: Observation,
        now_ms: u64,
    ) -> Result<(), AgentError> {
        let isolated = self.authority.is_isolated();
        let exit = obs.exit_code;
        let (finite, attempt_index, retry, declares_pub, has_readiness) = {
            let a = self.live.get_mut(&id).ok_or(AgentError::NotFound)?;
            a.last_obs = Some(obs);
            (
                a.lifecycle_finite,
                a.attempt_index,
                a.lifecycle.retry.clone(),
                a.lifecycle.declares_publication,
                a.lifecycle.readiness.is_some(),
            )
        };

        let running = self.live.get_mut(&id).and_then(|a| a.running.take());
        if let Some(r) = running {
            let prepared = r.into_prepared();
            let _ = self.driver.cleanup(prepared);
        }

        // Finite success → complete and clean (unless isolated).
        if finite && exit == Some(0) {
            if let Some(a) = self.live.get_mut(&id) {
                a.phase = AttemptPhase::Terminal { exit_code: exit };
                a.terminal_reason = Some(TerminalReason::completed(exit));
            }
            if !isolated {
                self.completed.insert(id);
                self.completion_events.insert(id);
                return self.cleanup_live(id);
            }
            return Ok(());
        }

        // Failure or continuous exit: optional retry (no default retry).
        if retry.enabled() && attempt_index < retry.max_attempts && !isolated {
            let seed = u64::from(id.as_bytes()[15]).saturating_add(now_ms);
            let backoff = retry.backoff_ms(attempt_index, seed);
            if let Some(a) = self.live.get_mut(&id) {
                a.phase = AttemptPhase::AwaitingRestart {
                    after_ms: now_ms.saturating_add(backoff),
                };
                a.terminal_reason = Some(TerminalReason {
                    code: reasons::RETRY_SCHEDULED,
                    detail: format!("retry after {backoff}ms (attempt {attempt_index})"),
                    exit_code: exit,
                    attempt_index,
                });
                a.attempt_index = attempt_index.saturating_add(1);
                a.ready = if has_readiness { Some(false) } else { None };
                if declares_pub {
                    a.publication_eligible = Some(false);
                }
            }
            return Ok(());
        }

        if let Some(a) = self.live.get_mut(&id) {
            a.phase = AttemptPhase::PermanentFailure;
            a.terminal_reason = Some(TerminalReason::permanent(
                if finite {
                    "finite workload failed without remaining retries"
                } else {
                    "continuous workload exited; no remaining retries"
                },
                exit,
                attempt_index,
            ));
        }
        Ok(())
    }

    fn maybe_restart(&mut self, id: AttemptId, now_ms: u64) -> Result<(), AgentError> {
        let after = match self.live.get(&id).map(|a| &a.phase) {
            Some(AttemptPhase::AwaitingRestart { after_ms }) => *after_ms,
            _ => return Ok(()),
        };
        if now_ms < after {
            return Ok(());
        }
        let p = {
            let a = self.live.get(&id).ok_or(AgentError::NotFound)?;
            AcceptedPlacement {
                attempt_id: id,
                unit_id: a.unit_id,
                placement_fence: a.placement_fence,
                release_root: a.release_root.clone(),
                runtime: a.runtime.clone(),
                lifecycle_finite: a.lifecycle_finite,
                capsule_verified: a.capsule_verified,
                lifecycle: a.lifecycle.clone(),
                hiccup: a.hiccup.clone(),
            }
        };
        let next_index = self.live.get(&id).map(|a| a.attempt_index).unwrap_or(1);
        // Remove stale live shell before start_placement re-inserts.
        self.live.remove(&id);
        self.start_placement(&p, now_ms, next_index)
    }

    fn tick_checks(
        &mut self,
        id: AttemptId,
        now_ms: u64,
        budget: &mut CheckBudget,
    ) -> Result<(), AgentError> {
        let Some(a) = self.live.get(&id) else {
            return Ok(());
        };
        if !matches!(a.phase, AttemptPhase::Running) {
            return Ok(());
        }
        let process_running = a.last_obs.as_ref().map(|o| o.running).unwrap_or(true);
        let started_ms = a.started_ms;
        let readiness_spec = a.lifecycle.readiness.clone();
        let liveness_spec = a.lifecycle.liveness.clone();
        let declares_pub = a.lifecycle.declares_publication;

        if let Some(spec) = readiness_spec {
            let due = self
                .live
                .get(&id)
                .map(|a| a.readiness.due(now_ms, started_ms, &spec))
                .unwrap_or(false);
            if due {
                let hiccup_bind = self.live.get(&id).and_then(|a| a.hiccup.clone());
                let unit_id = self.live.get(&id).map(|a| a.unit_id);
                let fence = self.live.get(&id).map(|a| a.placement_fence).unwrap_or(0);
                let out = match (spec.kind, hiccup_bind.as_ref(), unit_id) {
                    (CheckKind::Http, Some(bind), Some(unit)) => {
                        self.run_http_with_hiccup(id, &spec, bind, unit, fence, now_ms, budget)
                    }
                    _ => run_check(&spec, process_running, budget),
                };
                if let Some(a) = self.live.get_mut(&id) {
                    a.readiness.last_run_ms = now_ms;
                    a.readiness.record(out.ok, &spec);
                    a.ready = Some(a.readiness.passed);
                    if declares_pub {
                        a.publication_eligible = Some(a.readiness.passed);
                    }
                }
            }
        }

        if let Some(spec) = liveness_spec {
            let due = self
                .live
                .get(&id)
                .map(|a| a.liveness.due(now_ms, started_ms, &spec))
                .unwrap_or(false);
            if due {
                let out = run_check(&spec, process_running, budget);
                if let Some(a) = self.live.get_mut(&id) {
                    a.liveness.last_run_ms = now_ms;
                    a.liveness.record(out.ok, &spec);
                    if a.liveness.consecutive_failure >= spec.failure_threshold.max(1) {
                        // Liveness failure → treat as policy failure (stop child).
                        a.terminal_reason = Some(TerminalReason {
                            code: reasons::LIVENESS_FAILED,
                            detail: out.detail,
                            exit_code: None,
                            attempt_index: a.attempt_index,
                        });
                    }
                }
                let failed = self
                    .live
                    .get(&id)
                    .map(|a| {
                        a.terminal_reason
                            .as_ref()
                            .map(|t| t.code == reasons::LIVENESS_FAILED)
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);
                if failed {
                    if let Some(a) = self.live.get_mut(&id) {
                        if let Some(r) = a.running.as_mut() {
                            let _ = self.driver.kill(r);
                        }
                    }
                    let obs = Observation {
                        running: false,
                        exit_code: None,
                    };
                    self.apply_exit_policy(id, obs, now_ms)?;
                }
            }
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
        let (fence, grace_ms) = self
            .live
            .get(&id)
            .map(|a| (a.placement_fence, a.lifecycle.stop_grace_ms.max(1)))
            .ok_or(AgentError::NotFound)?;
        allow_effect(&self.authority, fence, EffectKind::Stop)?;
        if let Some(a) = self.live.get_mut(&id) {
            if let Some(r) = a.running.as_mut() {
                let _ = self
                    .driver
                    .terminate(r, Duration::from_millis(grace_ms.min(5_000)));
                let obs = self.driver.observe(r)?;
                if obs.running {
                    self.driver.kill(r)?;
                }
            }
            a.phase = AttemptPhase::Stopped;
            a.terminal_reason = Some(TerminalReason {
                code: reasons::STOP_SIGNAL,
                detail: "stopped by intent change or cancel".into(),
                exit_code: None,
                attempt_index: a.attempt_index,
            });
        }
        self.cleanup_live(id)
    }

    fn force_stop_cleanup(&mut self, id: AttemptId) -> Result<(), AgentError> {
        if let Some(a) = self.live.get_mut(&id) {
            if let Some(r) = a.running.as_mut() {
                let _ = self.driver.kill(r);
            }
        }
        self.cleanup_live(id)
    }

    fn cleanup_live(&mut self, id: AttemptId) -> Result<(), AgentError> {
        self.hiccup.remove_attempt(id);
        let Some(mut a) = self.live.remove(&id) else {
            return Ok(());
        };
        if let Some(running) = a.running.take() {
            let prepared = running.into_prepared();
            self.driver.cleanup(prepared)?;
        } else if a.attempt_root.exists() {
            remove_attempt_root(&a.attempt_root)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn run_http_with_hiccup(
        &mut self,
        id: AttemptId,
        spec: &crate::lifecycle::CheckSpec,
        placement: &HiccupPlacement,
        unit_id: UnitId,
        fence: u64,
        now_ms: u64,
        budget: &CheckBudget,
    ) -> crate::checks::CheckOutcome {
        if budget.exhausted() {
            return crate::checks::CheckOutcome {
                ok: false,
                reason_code: reasons::CHECK_SKIPPED_BUDGET,
                detail: "check skipped: reconcile budget exhausted".into(),
                elapsed_ms: 0,
            };
        }
        let timeout =
            std::time::Duration::from_millis(spec.timeout_ms.max(1)).min(budget.remaining());
        let start = std::time::Instant::now();
        let plan = match self.hiccup.plan(id) {
            Some(OutboundHealth::Post {
                authorization,
                body,
                content_type,
            }) => HttpRequestPlan::Post {
                authorization,
                content_type,
                body,
            },
            _ => HttpRequestPlan::GetOffer,
        };
        let exchange = match http_exchange(spec.target.as_deref(), timeout, &plan) {
            Ok(e) => e,
            Err(detail) => {
                return crate::checks::CheckOutcome {
                    ok: false,
                    reason_code: reasons::CHECK_TIMEOUT,
                    detail,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                };
            }
        };
        if exchange.ok {
            let _ = self.hiccup.on_health_ok(HealthOkCtx {
                placement,
                unit_id,
                attempt_id: id,
                fence,
                content_type: exchange.content_type.as_deref(),
                body: &exchange.body,
                health_interval_ms: spec.interval_ms,
                now_ms,
            });
        }
        let elapsed_ms = start.elapsed().as_millis() as u64;
        if exchange.ok {
            crate::checks::CheckOutcome {
                ok: true,
                reason_code: "lifecycle.check_ok",
                detail: "check succeeded".into(),
                elapsed_ms,
            }
        } else {
            crate::checks::CheckOutcome {
                ok: false,
                reason_code: reasons::READINESS_FAILED,
                detail: format!("http status {}", exchange.status),
                elapsed_ms,
            }
        }
    }

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
