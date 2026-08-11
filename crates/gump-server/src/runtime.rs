//! Controller → scheduler → agent execution composition for the local node.
//!
//! The controller reads only committed desired bindings. It then re-opens the
//! immutable Capsule, verifies it, decodes public declarations, materializes
//! the archive, schedules units, and hands fenced placements to the agent.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, TryLockError};

use gump_agent::{
    AcceptedPlacement, AuthorityState, CheckKind, CheckSpec, EffectExecutor, LifecycleContract,
    PipeSinkFactory, RetryPolicy, SecretPlanProvider,
};
use gump_capsule::archive::{ExtractLimits, materialize_application_archive};
use gump_capsule::{GumpCapsuleMeta, SegmentDescriptor, SegmentType, StreamingCapsuleReader};
use gump_connectors::{ByteRange, ObjectStore, RuntimeObjectStore, final_capsule_key};
use gump_crypto::{SealedDek, build_protected_aad, hpke_info, open_protected};
use gump_driver::{
    DeliveryScope, DriverKind, FdReferenceValue, InjectForm, NativeDriver, PipeChunkSink,
    RuntimeSpec, ScriptDriver, SecretPlan, SecretValue, StreamKind,
};
use gump_memory::{DesiredSnapshotEntry, MemoryCluster, RaftCommand, RaftResponse};
use gump_protocol::pb::{
    CheckKind as PbCheckKind, CheckSpecV1, DriverKind as PbDriverKind, InjectionKind,
    KeyEnvelopeV1, ProtectedConfigV1, ReleaseMetadataV1, RuntimeVariableV1, WorkloadLifetime,
};
use gump_scheduler::{
    CapabilityReport, NodeResources, PlacementController, PlacementOutcome, ProtectionLevel,
    WorkloadRequirements,
};
use gump_telemetry::TelemetryPlane;
use gump_types::ExecutionId;
use gump_types::{AttemptId, CapsuleId, ClusterId, NodeId, Secret, UnitId, WorkloadId};
use prost::Message;

use crate::custody::ClusterCustody;
use crate::deploy_txn::DesiredCapsuleBindingV1;
use crate::ringtail_relay::{RelayTarget, RingtailRelay};

const RECONCILE_FENCE: u64 = 1;
const RELEASE_FETCH_INITIAL_BACKOFF_MS: u64 = 1_000;
const RELEASE_FETCH_MAX_BACKOFF_MS: u64 = 60_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeStatus {
    pub desired: usize,
    pub placements: usize,
    pub completed: usize,
    pub ready: usize,
    pub hiccup_presence: usize,
    pub last_error: Option<String>,
    pub ringtail_active: bool,
    pub ringtail_accepted: u64,
    pub ringtail_failed: u64,
    pub ringtail_dropped: u64,
    pub s3_head_requests: u64,
    pub s3_full_get_requests: u64,
    pub s3_ranged_get_requests: u64,
    pub s3_bytes_read: u64,
}

struct SecretBinding {
    capsule_id: CapsuleId,
    workload_id: WorkloadId,
    unit: u32,
    node_id: u64,
    controller_epoch: u64,
    placement_fence: u64,
    capsule_meta: GumpCapsuleMeta,
    runtime_variables: BTreeMap<String, RuntimeVariableV1>,
    telemetry_sink: Option<TelemetrySinkContract>,
    telemetry_token: Option<Secret<Vec<u8>>>,
}

#[derive(Clone)]
struct TelemetrySinkContract {
    port: u16,
    path: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ReleaseIdentity {
    capsule_id: CapsuleId,
    content_digest: [u8; 32],
}

#[derive(Clone, Debug)]
struct ReleaseFetchFailure {
    attempts: u32,
    retry_after_ms: u64,
    error: String,
}

/// One node's live execution controller. No field is durable cluster state.
pub struct RuntimeCoordinator {
    cluster_id: ClusterId,
    node_id: NodeId,
    private_ip: Option<String>,
    memory_node_id: u64,
    state_root: PathBuf,
    scheduler: PlacementController,
    native: EffectExecutor<NativeDriver>,
    script: EffectExecutor<ScriptDriver>,
    secret_bindings: Arc<Mutex<BTreeMap<AttemptId, SecretBinding>>>,
    ringtail_relay: RingtailRelay,
    release_cache: BTreeMap<ReleaseIdentity, LoadedRelease>,
    release_failures: BTreeMap<ReleaseIdentity, ReleaseFetchFailure>,
    known_units: BTreeSet<UnitId>,
    status: RuntimeStatus,
}

impl std::fmt::Debug for RuntimeCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeCoordinator")
            .field("cluster_id", &self.cluster_id)
            .field("node_id", &self.node_id)
            .field("state_root", &self.state_root)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl RuntimeCoordinator {
    pub fn new(
        cluster_id: ClusterId,
        memory_node_id: u64,
        private_ip: Option<String>,
        state_root: PathBuf,
        store: Arc<Mutex<RuntimeObjectStore>>,
        custody: Arc<Mutex<ClusterCustody>>,
        telemetry: Option<Arc<Mutex<TelemetryPlane>>>,
    ) -> Result<Self, String> {
        let node_id = stable_id::<NodeId>(&[cluster_id.as_bytes(), &memory_node_id.to_be_bytes()])?;
        let attempts = state_root.join("attempts");
        std::fs::create_dir_all(&attempts)
            .map_err(|e| format!("create attempts root {}: {e}", attempts.display()))?;
        let authority = AuthorityState::connected(RECONCILE_FENCE, RECONCILE_FENCE);
        let bindings = Arc::new(Mutex::new(BTreeMap::new()));
        let secret_provider = secret_provider(
            cluster_id,
            Arc::clone(&store),
            Arc::clone(&custody),
            Arc::clone(&bindings),
        );
        let ringtail_relay = RingtailRelay::new();
        let mut native = EffectExecutor::new(
            NativeDriver::new(),
            attempts.join("native"),
            authority.clone(),
        )
        .with_secret_provider(Arc::clone(&secret_provider));
        let mut script =
            EffectExecutor::new(ScriptDriver::new(), attempts.join("script"), authority)
                .with_secret_provider(secret_provider);
        if let Some(plane) = telemetry {
            let factory = pipe_factory(plane, ringtail_relay.clone(), cluster_id, node_id);
            native = native.with_pipe_sink_factory(Arc::clone(&factory));
            script = script.with_pipe_sink_factory(factory);
        }

        let mut scheduler = PlacementController::new();
        scheduler
            .upsert_report(local_capabilities(node_id))
            .map_err(|e| e.detail)?;
        Ok(Self {
            cluster_id,
            node_id,
            private_ip,
            memory_node_id,
            state_root,
            scheduler,
            native,
            script,
            secret_bindings: bindings,
            ringtail_relay,
            release_cache: BTreeMap::new(),
            release_failures: BTreeMap::new(),
            known_units: BTreeSet::new(),
            status: RuntimeStatus {
                desired: 0,
                placements: 0,
                completed: 0,
                ready: 0,
                hiccup_presence: 0,
                last_error: None,
                ringtail_active: false,
                ringtail_accepted: 0,
                ringtail_failed: 0,
                ringtail_dropped: 0,
                s3_head_requests: 0,
                s3_full_get_requests: 0,
                s3_ranged_get_requests: 0,
                s3_bytes_read: 0,
            },
        })
    }

    pub fn status(&self) -> RuntimeStatus {
        self.status.clone()
    }

    fn load_release_cached(
        &mut self,
        identity: ReleaseIdentity,
        desired: &DesiredSnapshotEntry,
        store: &Arc<Mutex<RuntimeObjectStore>>,
        now_ms: u64,
    ) -> Result<LoadedRelease, String> {
        if let Some(failure) = self.release_failures.get(&identity) {
            if now_ms < failure.retry_after_ms {
                return Err(format!(
                    "Capsule {} fetch deferred until {} after {} failed attempt(s): {}",
                    identity.capsule_id, failure.retry_after_ms, failure.attempts, failure.error
                ));
            }
        }
        if let Some(loaded) = self.release_cache.get(&identity) {
            return Ok(loaded.clone());
        }

        match load_release(
            self.cluster_id,
            identity.capsule_id,
            desired,
            &self.state_root,
            store,
        ) {
            Ok(loaded) => {
                self.release_failures.remove(&identity);
                self.release_cache.insert(identity, loaded.clone());
                Ok(loaded)
            }
            Err(error) => {
                self.record_release_failure(identity, now_ms, &error);
                Err(error)
            }
        }
    }

    fn ensure_release_materialized(
        &mut self,
        identity: ReleaseIdentity,
        loaded: &LoadedRelease,
        store: &Arc<Mutex<RuntimeObjectStore>>,
        now_ms: u64,
    ) -> Result<(), String> {
        if loaded.release_root.is_dir() {
            self.release_failures.remove(&identity);
            return Ok(());
        }
        match materialize_release(loaded, &self.state_root, store) {
            Ok(()) => {
                self.release_failures.remove(&identity);
                Ok(())
            }
            Err(error) => {
                self.record_release_failure(identity, now_ms, &error);
                Err(error)
            }
        }
    }

    fn record_release_failure(&mut self, identity: ReleaseIdentity, now_ms: u64, error: &str) {
        let attempts = self
            .release_failures
            .get(&identity)
            .map(|failure| failure.attempts.saturating_add(1))
            .unwrap_or(1);
        let exponent = attempts.saturating_sub(1).min(6);
        let delay = RELEASE_FETCH_INITIAL_BACKOFF_MS
            .saturating_mul(1u64 << exponent)
            .min(RELEASE_FETCH_MAX_BACKOFF_MS);
        self.release_failures.insert(
            identity,
            ReleaseFetchFailure {
                attempts,
                retry_after_ms: now_ms.saturating_add(delay),
                error: error.to_string(),
            },
        );
    }

    pub fn reconcile(
        &mut self,
        cluster: &MemoryCluster,
        store: &Arc<Mutex<RuntimeObjectStore>>,
        now_ms: u64,
    ) -> Result<RuntimeStatus, String> {
        self.refresh_s3_stats(store);
        let desired = cluster.observed_desired_snapshot();
        let voters = cluster.voter_ids();
        let local_memory_id = self.memory_node_id;
        let mut native = Vec::new();
        let mut script = Vec::new();
        let mut current_units = BTreeSet::new();
        let mut current_attempts = BTreeSet::new();
        let mut current_releases = BTreeSet::new();
        let mut completion_targets = BTreeMap::new();
        let mut completed = 0usize;

        for entry in &desired {
            let binding: DesiredCapsuleBindingV1 =
                serde_json::from_slice(&entry.payload).map_err(|e| {
                    format!(
                        "decode desired binding for {}/{}: {e}",
                        entry.namespace, entry.app
                    )
                })?;
            if binding.schema != "gump.desired-capsule/1" {
                return Err(format!(
                    "unsupported desired binding schema {}",
                    binding.schema
                ));
            }
            let capsule_id: CapsuleId = binding
                .capsule_id
                .parse()
                .map_err(|_| "desired capsule_id is not UUIDv7".to_string())?;
            let identity = ReleaseIdentity {
                capsule_id,
                content_digest: entry.content_digest,
            };
            current_releases.insert(identity);
            let loaded = self.load_release_cached(identity, entry, store, now_ms)?;
            let units = if loaded.all_nodes {
                u32::try_from(voters.len()).unwrap_or(u32::MAX)
            } else {
                loaded.units
            };
            for unit_index in 0..units {
                // A unit is the stable logical slot of a workload. Release
                // generation belongs to execution/attempt identity; including
                // it here prevents consumers from superseding replaced
                // attempts for the same slot.
                let unit_id = stable_unit_id(loaded.workload_id, unit_index)?;
                if loaded.lifecycle_finite
                    && cluster.observed_finite_completed(
                        &entry.namespace,
                        &entry.app,
                        entry.generation,
                        unit_id.as_bytes(),
                    )
                {
                    completed = completed.saturating_add(1);
                    continue;
                }
                if voters
                    .get(unit_index as usize % voters.len().max(1))
                    .copied()
                    != Some(local_memory_id)
                {
                    continue;
                }
                self.ensure_release_materialized(identity, &loaded, store, now_ms)?;
                current_units.insert(unit_id);
                if !self.known_units.contains(&unit_id) {
                    let outcome = self.scheduler.place(&WorkloadRequirements {
                        workload_id: loaded.workload_id,
                        unit_id,
                        arch: std::env::consts::ARCH.into(),
                        driver: loaded.driver_name.into(),
                        required_enforced: loaded.required_enforced.clone(),
                        request: loaded.resources,
                        requires_port: loaded.requires_port,
                        lifecycle_finite: loaded.lifecycle_finite,
                    });
                    if !matches!(outcome, PlacementOutcome::Scheduled(_)) {
                        return Err(format!("unit {unit_id} is unschedulable: {outcome:?}"));
                    }
                    self.known_units.insert(unit_id);
                }
                let attempt_id = stable_id::<AttemptId>(&[
                    unit_id.as_bytes(),
                    capsule_id.as_bytes(),
                    b"attempt-1",
                ])?;
                current_attempts.insert(attempt_id);
                let execution_id = stable_id::<ExecutionId>(&[
                    loaded.workload_id.as_bytes(),
                    &entry.generation.to_be_bytes(),
                ])?;
                if loaded.lifecycle_finite {
                    completion_targets.insert(
                        attempt_id,
                        (
                            entry.namespace.clone(),
                            entry.app.clone(),
                            entry.generation,
                            unit_id,
                        ),
                    );
                }
                self.secret_bindings
                    .lock()
                    .map_err(|_| "secret binding lock poisoned".to_string())?
                    .entry(attempt_id)
                    .or_insert_with(|| SecretBinding {
                        capsule_id,
                        workload_id: loaded.workload_id,
                        unit: unit_index,
                        node_id: self.memory_node_id,
                        controller_epoch: RECONCILE_FENCE,
                        placement_fence: RECONCILE_FENCE,
                        capsule_meta: loaded.capsule_meta.clone(),
                        runtime_variables: loaded.runtime_variables.clone(),
                        telemetry_sink: loaded.telemetry_sink.clone(),
                        telemetry_token: None,
                    });
                let placement = AcceptedPlacement {
                    attempt_id,
                    unit_id,
                    placement_fence: RECONCILE_FENCE,
                    release_root: loaded.release_root.clone(),
                    runtime: loaded.runtime.clone(),
                    lifecycle_finite: loaded.lifecycle_finite,
                    capsule_verified: true,
                    lifecycle: loaded.lifecycle.clone(),
                    hiccup: loaded.hiccup.then(|| gump_agent::HiccupPlacement {
                        cluster_id: self.cluster_id,
                        namespace: loaded.namespace.clone(),
                        app_id: loaded.app_id.clone(),
                        workload_id: loaded.workload_id,
                        capsule_id,
                        execution_id,
                        node_id: self.node_id,
                        agent_incarnation: 1,
                        private_ip: self.private_ip.clone(),
                        named_publish: loaded
                            .telemetry_sink
                            .as_ref()
                            .map(|_| {
                                BTreeSet::from(["telemetry/sink/ratatouille-http".to_string()])
                            })
                            .unwrap_or_default(),
                        named_listen: BTreeSet::new(),
                        bind_liveness: loaded.hiccup_binding_liveness,
                    }),
                };
                match loaded.runtime.kind {
                    DriverKind::Native => native.push(placement),
                    DriverKind::Script => script.push(placement),
                }
            }
        }

        let obsolete: Vec<UnitId> = self
            .known_units
            .difference(&current_units)
            .copied()
            .collect();
        for unit in obsolete {
            let _ = self.scheduler.ledger.release(unit);
            self.known_units.remove(&unit);
        }
        self.secret_bindings
            .lock()
            .map_err(|_| "secret binding lock poisoned".to_string())?
            .retain(|attempt, _| current_attempts.contains(attempt));
        self.release_cache
            .retain(|identity, _| current_releases.contains(identity));
        self.release_failures
            .retain(|identity, _| current_releases.contains(identity));
        let native_reports = self
            .native
            .reconcile(&native, now_ms)
            .map_err(|e| e.to_string())?;
        let script_reports = self
            .script
            .reconcile(&script, now_ms)
            .map_err(|e| e.to_string())?;
        self.exchange_hiccup(cluster, now_ms)?;
        self.refresh_ringtail_relay();
        let native_completion_events = self.native.completion_events();
        let script_completion_events = self.script.completion_events();
        for (kind, attempt_id) in native_completion_events
            .into_iter()
            .map(|id| (DriverKind::Native, id))
            .chain(
                script_completion_events
                    .into_iter()
                    .map(|id| (DriverKind::Script, id)),
            )
        {
            let (namespace, app, generation, unit_id) = completion_targets
                .get(&attempt_id)
                .ok_or_else(|| format!("completion target missing for attempt {attempt_id}"))?;
            match cluster.client_write(RaftCommand::CompleteFinite {
                namespace: namespace.clone(),
                app: app.clone(),
                generation: *generation,
                unit_id: *unit_id.as_bytes(),
            })? {
                RaftResponse::Applied(_) | RaftResponse::Replay(_) => {
                    completed = completed.saturating_add(1);
                    match kind {
                        DriverKind::Native => self.native.acknowledge_completion(attempt_id),
                        DriverKind::Script => self.script.acknowledge_completion(attempt_id),
                    }
                }
                RaftResponse::Rejected(reason) => {
                    return Err(format!("commit finite completion: {reason}"));
                }
            }
        }
        let relay = self.ringtail_relay.stats();
        self.refresh_s3_stats(store);
        let ready = native_reports
            .iter()
            .chain(&script_reports)
            .filter(|report| report.ready == Some(true))
            .count();
        let hiccup_presence = self
            .native
            .hiccup_plane()
            .board
            .presence_count()
            .saturating_add(self.script.hiccup_plane().board.presence_count());
        self.status = RuntimeStatus {
            desired: desired.len(),
            placements: native_reports.len() + script_reports.len(),
            completed,
            ready,
            hiccup_presence,
            last_error: None,
            ringtail_active: relay.active,
            ringtail_accepted: relay.accepted,
            ringtail_failed: relay.failed,
            ringtail_dropped: relay.dropped,
            s3_head_requests: self.status.s3_head_requests,
            s3_full_get_requests: self.status.s3_full_get_requests,
            s3_ranged_get_requests: self.status.s3_ranged_get_requests,
            s3_bytes_read: self.status.s3_bytes_read,
        };
        Ok(self.status.clone())
    }

    pub fn note_error(&mut self, error: String) {
        self.status.last_error = Some(error);
    }

    fn refresh_s3_stats(&mut self, store: &Arc<Mutex<RuntimeObjectStore>>) {
        let Some(stats) = store.lock().ok().and_then(|store| store.s3_read_stats()) else {
            return;
        };
        self.status.s3_head_requests = stats.head_requests;
        self.status.s3_full_get_requests = stats.full_get_requests;
        self.status.s3_ranged_get_requests = stats.ranged_get_requests;
        self.status.s3_bytes_read = stats.bytes_read;
    }

    fn refresh_ringtail_relay(&self) {
        let Ok(bindings) = self.secret_bindings.lock() else {
            self.ringtail_relay.set_target(None);
            return;
        };
        let target = bindings.iter().find_map(|(attempt, binding)| {
            self.native
                .hiccup_plane()
                .board
                .presence_for_attempt(*attempt)?;
            let sink = binding.telemetry_sink.as_ref()?;
            let token = binding.telemetry_token.as_ref()?;
            let token = std::str::from_utf8(token.expose()).ok()?.to_string();
            Some(RelayTarget {
                address: ([127, 0, 0, 1], sink.port).into(),
                path: sink.path.clone(),
                token: Secret::new(token),
            })
        });
        self.ringtail_relay.set_target(target);
    }

    fn exchange_hiccup(&mut self, cluster: &MemoryCluster, now_ms: u64) -> Result<(), String> {
        let native = self.native.hiccup_cluster_snapshot(self.node_id, now_ms)?;
        let script = self.script.hiccup_cluster_snapshot(self.node_id, now_ms)?;
        let payload = gump_hiccup::combine_cluster_snapshots(self.node_id, &[&native, &script])?;
        for snapshot in cluster.exchange_hiccup_snapshot(payload)? {
            self.native
                .merge_hiccup_cluster_snapshot(&snapshot, now_ms)?;
            self.script
                .merge_hiccup_cluster_snapshot(&snapshot, now_ms)?;
        }
        Ok(())
    }
}

#[derive(Clone)]
struct LoadedRelease {
    cluster_id: ClusterId,
    capsule_id: CapsuleId,
    capsule_meta: GumpCapsuleMeta,
    runtime_variables: BTreeMap<String, RuntimeVariableV1>,
    workload_id: WorkloadId,
    namespace: String,
    app_id: String,
    hiccup: bool,
    hiccup_binding_liveness: bool,
    telemetry_sink: Option<TelemetrySinkContract>,
    units: u32,
    all_nodes: bool,
    driver_name: &'static str,
    required_enforced: Vec<String>,
    resources: NodeResources,
    requires_port: bool,
    lifecycle_finite: bool,
    lifecycle: LifecycleContract,
    runtime: RuntimeSpec,
    release_root: PathBuf,
}

fn load_release(
    cluster_id: ClusterId,
    capsule_id: CapsuleId,
    desired: &DesiredSnapshotEntry,
    state_root: &Path,
    store: &Arc<Mutex<RuntimeObjectStore>>,
) -> Result<LoadedRelease, String> {
    let key = final_capsule_key(cluster_id, capsule_id).map_err(|e| e.to_string())?;
    let guard = store
        .lock()
        .map_err(|_| "object store lock poisoned".to_string())?;
    let evidence = guard.head(&key).map_err(|e| e.to_string())?;
    if evidence.digest != desired.content_digest {
        return Err("committed desired digest differs from immutable Capsule".into());
    }
    let meta =
        StreamingCapsuleReader::new(guard.get_reader(&key, None).map_err(|e| e.to_string())?)
            .verify()
            .map_err(|e| format!("verify execution Capsule: {e}"))?;
    if meta.header.cluster_id != *cluster_id.as_bytes()
        || meta.header.capsule_id != *capsule_id.as_bytes()
    {
        return Err("execution Capsule identity mismatch".into());
    }
    let public = read_segment(&*guard, &key, &meta, SegmentType::PublicMetadata)?;
    let release = ReleaseMetadataV1::decode(public.as_slice())
        .map_err(|e| format!("decode ReleaseMetadataV1: {e}"))?;
    let runtime_variables = release
        .runtime_variables
        .iter()
        .cloned()
        .map(|variable| (variable.logical_name.clone(), variable))
        .collect();
    let manifest = release
        .normalized_manifest
        .ok_or("Capsule lacks normalized manifest")?;
    let app = manifest.app.ok_or("manifest lacks app identity")?;
    let workload_bytes = app.workload_id.ok_or("manifest lacks stable workload_id")?;
    let workload_id = parse_id::<WorkloadId>(&workload_bytes, "workload_id")?;
    let workload = manifest
        .workload
        .ok_or("manifest lacks workload declaration")?;
    let runtime_pb = manifest
        .runtime
        .ok_or("manifest lacks runtime declaration")?;
    let kind = match PbDriverKind::try_from(runtime_pb.driver).ok() {
        Some(PbDriverKind::Native) => DriverKind::Native,
        Some(PbDriverKind::Script) => DriverKind::Script,
        Some(PbDriverKind::Oci) => return Err("OCI execution driver is not installed".into()),
        _ => return Err("runtime driver is unspecified".into()),
    };
    let release_root = state_root.join("apps").join(capsule_id.to_hyphenated());
    let resources = manifest.resources.unwrap_or_default();
    let required_enforced = resources.capabilities.clone();
    let lifecycle_finite =
        WorkloadLifetime::try_from(workload.lifetime).ok() == Some(WorkloadLifetime::Finite);
    let readiness = manifest
        .health
        .as_ref()
        .and_then(|h| h.readiness.as_ref())
        .map(|spec| check_spec(spec, &runtime_pb.ports))
        .transpose()?
        .flatten();
    let liveness = manifest
        .health
        .as_ref()
        .and_then(|h| h.liveness.as_ref())
        .map(|spec| check_spec(spec, &runtime_pb.ports))
        .transpose()?
        .flatten();
    let completion = manifest
        .health
        .as_ref()
        .and_then(|h| h.completion.as_ref())
        .map(|spec| check_spec(spec, &runtime_pb.ports))
        .transpose()?
        .flatten();
    let hiccup = manifest.hiccup.is_some()
        || readiness
            .as_ref()
            .is_some_and(|c| c.kind == CheckKind::Http)
        || liveness.as_ref().is_some_and(|c| c.kind == CheckKind::Http);
    let hiccup_binding_liveness = manifest
        .hiccup
        .as_ref()
        .and_then(|spec| spec.health_binding.as_deref())
        .map(|binding| binding == "liveness")
        .unwrap_or_else(|| readiness.is_none() && liveness.is_some());
    let telemetry_sink = manifest
        .provides
        .iter()
        .find(|capability| capability.name == "telemetry_sink")
        .map(|capability| -> Result<TelemetrySinkContract, String> {
            if capability.protocol != "ratatouille-http-ndjson/1"
                || capability.authentication != "gump-attempt-bearer"
                || capability.scope.as_deref() != Some("node")
            {
                return Err("telemetry_sink capability does not match gump-ringtail/1".into());
            }
            let port = runtime_pb
                .ports
                .iter()
                .find(|port| port.name == capability.port_name)
                .and_then(|port| port.fixed_port)
                .ok_or("telemetry_sink requires a currently resolvable fixed named port")?;
            Ok(TelemetrySinkContract {
                port: port
                    .try_into()
                    .map_err(|_| "telemetry_sink port is out of range")?,
                path: capability.path.clone().unwrap_or_else(|| "/sink".into()),
            })
        })
        .transpose()?;
    let lifecycle = LifecycleContract {
        readiness,
        liveness,
        completion,
        retry: RetryPolicy {
            max_attempts: workload.max_attempts.unwrap_or(0),
            ..RetryPolicy::default()
        },
        declares_publication: manifest.publication.is_some(),
        stop_grace_ms: runtime_pb.stop_timeout_ms,
    };
    let units = manifest
        .deploy
        .as_ref()
        .and_then(|d| d.units)
        .unwrap_or(1)
        .clamp(1, 4096);
    let all_nodes = manifest
        .deploy
        .as_ref()
        .map(|d| d.coverage == gump_protocol::pb::CoverageKind::AllNodes as i32)
        .unwrap_or(false);
    Ok(LoadedRelease {
        cluster_id,
        capsule_id,
        capsule_meta: meta,
        runtime_variables,
        workload_id,
        namespace: app.namespace,
        app_id: app.app_id,
        hiccup,
        hiccup_binding_liveness,
        telemetry_sink,
        units,
        all_nodes,
        driver_name: match kind {
            DriverKind::Native => "native",
            DriverKind::Script => "script",
        },
        required_enforced,
        resources: NodeResources {
            millicores: resources
                .cpu_request_millis
                .unwrap_or(10)
                .try_into()
                .unwrap_or(u32::MAX),
            memory_bytes: resources.memory_request_bytes.unwrap_or(1024 * 1024),
            gpu_devices: resources.gpu_count.unwrap_or(0),
            ports: if runtime_pb.ports.is_empty() {
                0
            } else {
                runtime_pb.ports.len().try_into().unwrap_or(u32::MAX)
            },
        },
        requires_port: !runtime_pb.ports.is_empty(),
        lifecycle_finite,
        lifecycle,
        runtime: RuntimeSpec {
            kind,
            command: runtime_pb
                .command
                .into_iter()
                .map(|s| s.trim_start_matches("./").to_string())
                .collect(),
            interpreter: if runtime_pb.interpreter.is_empty() {
                None
            } else {
                Some(runtime_pb.interpreter)
            },
            workdir: nonempty(runtime_pb.workdir),
        },
        release_root,
    })
}

fn materialize_release(
    loaded: &LoadedRelease,
    state_root: &Path,
    store: &Arc<Mutex<RuntimeObjectStore>>,
) -> Result<(), String> {
    if loaded.release_root.is_dir() {
        return Ok(());
    }
    let key = final_capsule_key(loaded.cluster_id, loaded.capsule_id).map_err(|e| e.to_string())?;
    let descriptor = descriptor(&loaded.capsule_meta, SegmentType::ApplicationArchive)?;
    let start = loaded
        .capsule_meta
        .inner_file_offset
        .saturating_add(descriptor.offset);
    let guard = store
        .lock()
        .map_err(|_| "object store lock poisoned".to_string())?;
    let archive = guard
        .get_reader(
            &key,
            Some(ByteRange {
                start,
                end: Some(start.saturating_add(descriptor.stored_length)),
            }),
        )
        .map_err(|e| e.to_string())?;
    materialize_application_archive(
        state_root,
        loaded.capsule_id,
        archive,
        &ExtractLimits::default(),
    )
    .map(|_| ())
    .map_err(|e| format!("materialize Capsule: {e}"))
}

fn secret_provider(
    cluster_id: ClusterId,
    store: Arc<Mutex<RuntimeObjectStore>>,
    custody: Arc<Mutex<ClusterCustody>>,
    bindings: Arc<Mutex<BTreeMap<AttemptId, SecretBinding>>>,
) -> SecretPlanProvider {
    Arc::new(move |placement| {
        let mut bindings = bindings
            .lock()
            .map_err(|_| "secret binding lock poisoned".to_string())?;
        let binding = bindings
            .get_mut(&placement.attempt_id)
            .ok_or("missing secret binding")?;
        let key = final_capsule_key(cluster_id, binding.capsule_id).map_err(|e| e.to_string())?;
        let store = store
            .lock()
            .map_err(|_| "object store lock poisoned".to_string())?;
        let meta = binding.capsule_meta.clone();
        let vars = binding.runtime_variables.clone();
        let protected = read_segment(&*store, &key, &meta, SegmentType::ProtectedConfig)?;
        let envelope = KeyEnvelopeV1::decode(
            read_segment(&*store, &key, &meta, SegmentType::KeyEnvelope)?.as_slice(),
        )
        .map_err(|e| e.to_string())?;
        let encapsulated_key: [u8; 32] = envelope
            .hpke_encapsulated_key
            .as_slice()
            .try_into()
            .map_err(|_| "HPKE encapsulated key must be 32 bytes".to_string())?;
        let nonce: [u8; 24] = envelope
            .protected_nonce
            .as_slice()
            .try_into()
            .map_err(|_| "protected nonce must be 24 bytes".to_string())?;
        let pub_desc = descriptor(&meta, SegmentType::PublicMetadata)?;
        let archive_desc = descriptor(&meta, SegmentType::ApplicationArchive)?;
        let aad = build_protected_aad(
            binding.capsule_id.as_bytes(),
            cluster_id.as_bytes(),
            &pub_desc.digest,
            &archive_desc.digest,
        );
        if blake3::hash(&aad).as_bytes() != envelope.aad_digest.as_slice() {
            return Err("key envelope AAD digest mismatch".into());
        }
        let info = hpke_info(binding.capsule_id.as_bytes(), cluster_id.as_bytes());
        let sealed = SealedDek {
            encapsulated_key,
            wrapped_dek: envelope.wrapped_dek,
        };
        let dek = custody
            .lock()
            .map_err(|_| "custody lock poisoned".to_string())?
            .unwrap_dek(&envelope.cluster_key_id, &sealed, &info, &aad)
            .map_err(|e| e.to_string())?;
        let plaintext =
            open_protected(dek.expose(), &nonce, &aad, &protected).map_err(|e| e.to_string())?;
        let config =
            ProtectedConfigV1::decode(plaintext.expose().as_slice()).map_err(|e| e.to_string())?;
        let mut values = Vec::new();
        for value in config.values.into_iter().filter(|v| v.present) {
            let contract = vars
                .get(&value.logical_name)
                .ok_or("protected value lacks public contract")?;
            let form = match InjectionKind::try_from(value.injection).ok() {
                Some(InjectionKind::Env) => InjectForm::Env,
                Some(InjectionKind::Fd) => InjectForm::Fd {
                    fd: contract
                        .inherited_fd
                        .ok_or("FD variable lacks inherited_fd")?
                        .try_into()
                        .map_err(|_| "inherited_fd out of range")?,
                    reference_env: contract.reference_env.clone(),
                    reference_value: match contract.reference_value.as_str() {
                        "descriptor_number" => FdReferenceValue::DescriptorNumber,
                        _ => FdReferenceValue::ProcPath,
                    },
                },
                _ => return Err("unsupported secret injection kind".into()),
            };
            values.push(SecretValue {
                logical_name: value.logical_name,
                form,
                bytes: Secret::new(value.value),
            });
        }
        for contract in vars
            .values()
            .filter(|contract| contract.source_kind == "gump:attempt-token")
        {
            if values
                .iter()
                .any(|value| value.logical_name == contract.logical_name)
            {
                return Err("Gump-generated variable also has protected Capsule material".into());
            }
            let mut random = [0u8; 32];
            getrandom::fill(&mut random)
                .map_err(|e| format!("generate per-attempt credential: {e}"))?;
            let bytes = hex_encode(&random).into_bytes();
            if contract.max_bytes != 0 && bytes.len() as u64 > contract.max_bytes {
                return Err(format!(
                    "generated variable {} exceeds declared max_bytes",
                    contract.logical_name
                ));
            }
            let form = match InjectionKind::try_from(contract.injection).ok() {
                Some(InjectionKind::Env) => InjectForm::Env,
                Some(InjectionKind::Fd) => InjectForm::Fd {
                    fd: contract
                        .inherited_fd
                        .ok_or("generated FD variable lacks inherited_fd")?
                        .try_into()
                        .map_err(|_| "generated inherited_fd out of range")?,
                    reference_env: contract.reference_env.clone(),
                    reference_value: match contract.reference_value.as_str() {
                        "descriptor_number" => FdReferenceValue::DescriptorNumber,
                        _ => FdReferenceValue::ProcPath,
                    },
                },
                _ => return Err("unsupported generated secret injection kind".into()),
            };
            values.push(SecretValue {
                logical_name: contract.logical_name.clone(),
                form,
                bytes: Secret::new(bytes),
            });
            if contract.logical_name == "RINGTAIL_TOKEN_FD" && binding.telemetry_sink.is_some() {
                let token = values
                    .last()
                    .expect("generated value inserted")
                    .bytes
                    .expose()
                    .clone();
                binding.telemetry_token = Some(Secret::new(token));
            }
        }
        if placement.hiccup.is_some() {
            if values
                .iter()
                .any(|value| value.logical_name == gump_hiccup::TOKEN_FD_ENV)
            {
                return Err("GUMP_HICCUP_TOKEN_FD is reserved by Gump".into());
            }
            let used: BTreeSet<u16> = values
                .iter()
                .filter_map(|value| match value.form {
                    InjectForm::Fd { fd, .. } => Some(fd),
                    InjectForm::Env => None,
                })
                .collect();
            let fd = (64u16..=255)
                .rev()
                .find(|fd| !used.contains(fd))
                .ok_or("no inherited descriptor available for Hiccup token")?;
            let mut token = vec![0u8; gump_hiccup::TOKEN_BYTES];
            getrandom::fill(&mut token).map_err(|e| format!("generate Hiccup token: {e}"))?;
            values.push(SecretValue {
                logical_name: gump_hiccup::TOKEN_FD_ENV.into(),
                form: InjectForm::Fd {
                    fd,
                    reference_env: Some(gump_hiccup::TOKEN_FD_ENV.into()),
                    reference_value: FdReferenceValue::DescriptorNumber,
                },
                bytes: Secret::new(token),
            });
        }
        Ok(SecretPlan::scoped(
            DeliveryScope {
                cluster_id,
                workload_id: binding.workload_id,
                release_id: binding.capsule_id,
                unit: binding.unit,
                attempt_id: placement.attempt_id,
                node_id: binding.node_id,
                controller_epoch: binding.controller_epoch,
                placement_fence: binding.placement_fence,
            },
            values,
        ))
    })
}

struct TelemetrySink {
    plane: Arc<Mutex<TelemetryPlane>>,
    stdout: AtomicU64,
    stderr: AtomicU64,
    relay: RingtailRelay,
    cluster_id: ClusterId,
    node_id: NodeId,
    attempt_id: AttemptId,
}

impl PipeChunkSink for TelemetrySink {
    fn on_chunk(&self, kind: StreamKind, chunk: &[u8]) {
        let seq = match kind {
            StreamKind::Stdout => self.stdout.fetch_add(1, Ordering::Relaxed),
            StreamKind::Stderr => self.stderr.fetch_add(1, Ordering::Relaxed),
        };
        let topic = match kind {
            StreamKind::Stdout => "app/stdout",
            StreamKind::Stderr => "app/stderr",
        };
        match self.plane.try_lock() {
            Ok(mut plane) => {
                let _ = plane.ingest_application_topic(topic, chunk, seq);
            }
            Err(TryLockError::WouldBlock | TryLockError::Poisoned(_)) => {}
        }
        self.relay.try_emit(
            topic,
            seq,
            chunk,
            self.cluster_id.to_hyphenated(),
            self.node_id.to_hyphenated(),
            self.attempt_id.to_hyphenated(),
        );
    }
}

fn pipe_factory(
    plane: Arc<Mutex<TelemetryPlane>>,
    relay: RingtailRelay,
    cluster_id: ClusterId,
    node_id: NodeId,
) -> PipeSinkFactory {
    Arc::new(move |attempt_id| {
        Arc::new(TelemetrySink {
            plane: Arc::clone(&plane),
            stdout: AtomicU64::new(0),
            stderr: AtomicU64::new(0),
            relay: relay.clone(),
            cluster_id,
            node_id,
            attempt_id,
        })
    })
}

fn descriptor(meta: &GumpCapsuleMeta, ty: SegmentType) -> Result<&SegmentDescriptor, String> {
    meta.table
        .descriptors
        .iter()
        .find(|d| d.segment_type == ty)
        .ok_or_else(|| format!("Capsule lacks {ty:?} segment"))
}

fn read_segment<S: ObjectStore>(
    store: &S,
    key: &gump_connectors::ObjectKey,
    meta: &GumpCapsuleMeta,
    ty: SegmentType,
) -> Result<Vec<u8>, String> {
    let d = descriptor(meta, ty)?;
    let start = meta.inner_file_offset.saturating_add(d.offset);
    store
        .get(
            key,
            Some(ByteRange {
                start,
                end: Some(start.saturating_add(d.stored_length)),
            }),
        )
        .map_err(|e| e.to_string())
}

fn check_spec(
    spec: &CheckSpecV1,
    ports: &[gump_protocol::pb::PortSpecV1],
) -> Result<Option<CheckSpec>, String> {
    let kind = match PbCheckKind::try_from(spec.kind).ok() {
        Some(PbCheckKind::Process) => CheckKind::Process,
        Some(PbCheckKind::Tcp) => CheckKind::Tcp,
        Some(PbCheckKind::Http) => CheckKind::Http,
        Some(PbCheckKind::Command) => CheckKind::Command,
        _ => return Ok(None),
    };
    let target = match kind {
        CheckKind::Http | CheckKind::Tcp => {
            let name = spec
                .port_name
                .as_deref()
                .ok_or("network health check lacks port name")?;
            let port = ports
                .iter()
                .find(|port| port.name == name)
                .ok_or_else(|| format!("health check refers to unknown port {name}"))?;
            let number = port
                .fixed_port
                .ok_or_else(|| format!("automatic port {name} is not allocated yet"))?;
            let base = format!("127.0.0.1:{number}");
            Some(if kind == CheckKind::Http {
                format!("http://{base}{}", spec.path.as_deref().unwrap_or("/"))
            } else {
                base
            })
        }
        CheckKind::Process | CheckKind::Command => None,
    };
    Ok(Some(CheckSpec {
        kind,
        target,
        command: if spec.command.is_empty() {
            None
        } else {
            Some(spec.command.clone())
        },
        interval_ms: spec.interval_ms.max(1),
        timeout_ms: spec.timeout_ms.max(1),
        initial_delay_ms: spec.initial_delay_ms,
        success_threshold: spec.successes.max(1),
        failure_threshold: spec.failures.max(1),
        max_output_bytes: 4096,
    }))
}

fn local_capabilities(node_id: NodeId) -> CapabilityReport {
    let mut capabilities = BTreeMap::new();
    capabilities.insert("process-group".into(), ProtectionLevel::Enforced);
    CapabilityReport {
        node_id,
        revision: 1,
        placement_fence: RECONCILE_FENCE,
        arch: std::env::consts::ARCH.into(),
        drivers: vec!["native".into(), "script".into()],
        capabilities,
        allocatable: NodeResources {
            millicores: std::thread::available_parallelism()
                .map(|count| count.get().saturating_mul(1_000).min(u32::MAX as usize) as u32)
                .unwrap_or(1_000),
            memory_bytes: local_allocatable_memory_bytes(),
            gpu_devices: 0,
            ports: 16_384,
        },
        drained: false,
    }
}

fn local_allocatable_memory_bytes() -> u64 {
    // Advertise a conservative fraction of physical RAM. A failed platform
    // query must under-advertise rather than invent datacenter-sized capacity.
    let pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) };
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if pages <= 0 || page_size <= 0 {
        return 512 * 1024 * 1024;
    }
    (pages as u64)
        .saturating_mul(page_size as u64)
        .saturating_mul(9)
        / 10
}

fn stable_id<T>(parts: &[&[u8]]) -> Result<T, String>
where
    T: StableId,
{
    let mut h = blake3::Hasher::new();
    for part in parts {
        h.update(&(part.len() as u64).to_be_bytes());
        h.update(part);
    }
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&h.finalize().as_bytes()[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x70;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    T::from_bytes(bytes)
}

fn stable_unit_id(workload_id: WorkloadId, unit_index: u32) -> Result<UnitId, String> {
    stable_id::<UnitId>(&[workload_id.as_bytes(), &unit_index.to_be_bytes()])
}

trait StableId: Sized {
    fn from_bytes(bytes: [u8; 16]) -> Result<Self, String>;
}
macro_rules! stable {
    ($($t:ty),*) => {$(
        impl StableId for $t {
            fn from_bytes(bytes: [u8; 16]) -> Result<Self, String> {
                <$t>::from_bytes(bytes).map_err(|e| e.to_string())
            }
        }
    )*};
}
stable!(NodeId, UnitId, AttemptId, WorkloadId, ExecutionId);

fn parse_id<T: StableId>(bytes: &[u8], name: &str) -> Result<T, String> {
    let bytes: [u8; 16] = bytes
        .try_into()
        .map_err(|_| format!("{name} must be 16 bytes"))?;
    T::from_bytes(bytes)
}

fn nonempty(value: String) -> Option<String> {
    if value.is_empty() || value == "." {
        None
    } else {
        Some(value.trim_start_matches("./").to_string())
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod unit_identity_tests {
    use super::*;

    #[test]
    fn unit_slot_identity_is_stable_while_attempt_identity_changes() {
        let workload = WorkloadId::new();
        let unit = stable_unit_id(workload, 0).expect("unit");
        assert_eq!(unit, stable_unit_id(workload, 0).expect("same unit"));
        assert_ne!(unit, stable_unit_id(workload, 1).expect("next unit"));

        let first_capsule = CapsuleId::new();
        let replacement_capsule = CapsuleId::new();
        let first_attempt =
            stable_id::<AttemptId>(&[unit.as_bytes(), first_capsule.as_bytes(), b"attempt-1"])
                .expect("first attempt");
        let replacement_attempt = stable_id::<AttemptId>(&[
            unit.as_bytes(),
            replacement_capsule.as_bytes(),
            b"attempt-1",
        ])
        .expect("replacement attempt");
        assert_ne!(first_attempt, replacement_attempt);
    }
}
