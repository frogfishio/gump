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
    DeliveryScope, DriverKind, InjectForm, NativeDriver, PipeChunkSink, RuntimeSpec, ScriptDriver,
    SecretPlan, SecretValue, StreamKind,
};
use gump_memory::{DesiredSnapshotEntry, MemoryCluster, RaftCommand, RaftResponse};
use gump_protocol::pb::{
    CheckKind as PbCheckKind, CheckSpecV1, DriverKind as PbDriverKind, InjectionKind,
    KeyEnvelopeV1, ProtectedConfigV1, ReleaseMetadataV1, WorkloadLifetime,
};
use gump_scheduler::{
    CapabilityReport, NodeResources, PlacementController, PlacementOutcome, ProtectionLevel,
    WorkloadRequirements,
};
use gump_telemetry::TelemetryPlane;
use gump_types::{AttemptId, CapsuleId, ClusterId, NodeId, Secret, UnitId, WorkloadId};
use prost::Message;

use crate::custody::ClusterCustody;
use crate::deploy_txn::DesiredCapsuleBindingV1;

const RECONCILE_FENCE: u64 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeStatus {
    pub desired: usize,
    pub placements: usize,
    pub completed: usize,
    pub last_error: Option<String>,
}

struct SecretBinding {
    capsule_id: CapsuleId,
    workload_id: WorkloadId,
    unit: u32,
    node_id: u64,
    controller_epoch: u64,
    placement_fence: u64,
}

/// One node's live execution controller. No field is durable cluster state.
pub struct RuntimeCoordinator {
    cluster_id: ClusterId,
    node_id: NodeId,
    memory_node_id: u64,
    state_root: PathBuf,
    scheduler: PlacementController,
    native: EffectExecutor<NativeDriver>,
    script: EffectExecutor<ScriptDriver>,
    secret_bindings: Arc<Mutex<BTreeMap<AttemptId, SecretBinding>>>,
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
            let factory = pipe_factory(plane);
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
            memory_node_id,
            state_root,
            scheduler,
            native,
            script,
            secret_bindings: bindings,
            known_units: BTreeSet::new(),
            status: RuntimeStatus {
                desired: 0,
                placements: 0,
                completed: 0,
                last_error: None,
            },
        })
    }

    pub fn status(&self) -> RuntimeStatus {
        self.status.clone()
    }

    pub fn reconcile(
        &mut self,
        cluster: &MemoryCluster,
        store: &Arc<Mutex<RuntimeObjectStore>>,
        now_ms: u64,
    ) -> Result<RuntimeStatus, String> {
        let desired = cluster.observed_desired_snapshot();
        let voters = cluster.voter_ids();
        let local_memory_id = self.memory_node_id;
        let mut native = Vec::new();
        let mut script = Vec::new();
        let mut current_units = BTreeSet::new();
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
            let loaded = load_release(self.cluster_id, capsule_id, entry, &self.state_root, store)?;
            let units = if loaded.all_nodes {
                u32::try_from(voters.len()).unwrap_or(u32::MAX)
            } else {
                loaded.units
            };
            for unit_index in 0..units {
                let unit_id = stable_id::<UnitId>(&[
                    loaded.workload_id.as_bytes(),
                    &entry.generation.to_be_bytes(),
                    &unit_index.to_be_bytes(),
                ])?;
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
                    .insert(
                        attempt_id,
                        SecretBinding {
                            capsule_id,
                            workload_id: loaded.workload_id,
                            unit: unit_index,
                            node_id: self.memory_node_id,
                            controller_epoch: RECONCILE_FENCE,
                            placement_fence: RECONCILE_FENCE,
                        },
                    );
                let placement = AcceptedPlacement {
                    attempt_id,
                    unit_id,
                    placement_fence: RECONCILE_FENCE,
                    release_root: loaded.release_root.clone(),
                    runtime: loaded.runtime.clone(),
                    lifecycle_finite: loaded.lifecycle_finite,
                    capsule_verified: true,
                    lifecycle: loaded.lifecycle.clone(),
                    hiccup: None,
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
        let native_reports = self
            .native
            .reconcile(&native, now_ms)
            .map_err(|e| e.to_string())?;
        let script_reports = self
            .script
            .reconcile(&script, now_ms)
            .map_err(|e| e.to_string())?;
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
        self.status = RuntimeStatus {
            desired: desired.len(),
            placements: native_reports.len() + script_reports.len(),
            completed,
            last_error: None,
        };
        Ok(self.status.clone())
    }

    pub fn note_error(&mut self, error: String) {
        self.status.last_error = Some(error);
    }
}

struct LoadedRelease {
    workload_id: WorkloadId,
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
    if !release_root.is_dir() {
        let d = descriptor(&meta, SegmentType::ApplicationArchive)?;
        let start = meta.inner_file_offset.saturating_add(d.offset);
        let archive = guard
            .get_reader(
                &key,
                Some(ByteRange {
                    start,
                    end: Some(start.saturating_add(d.stored_length)),
                }),
            )
            .map_err(|e| e.to_string())?;
        materialize_application_archive(state_root, capsule_id, archive, &ExtractLimits::default())
            .map_err(|e| format!("materialize Capsule: {e}"))?;
    }
    let resources = manifest.resources.unwrap_or_default();
    let required_enforced = resources.capabilities.clone();
    let lifecycle_finite =
        WorkloadLifetime::try_from(workload.lifetime).ok() == Some(WorkloadLifetime::Finite);
    let lifecycle = LifecycleContract {
        readiness: manifest
            .health
            .as_ref()
            .and_then(|h| h.readiness.as_ref())
            .and_then(check_spec),
        liveness: manifest
            .health
            .as_ref()
            .and_then(|h| h.liveness.as_ref())
            .and_then(check_spec),
        completion: manifest
            .health
            .as_ref()
            .and_then(|h| h.completion.as_ref())
            .and_then(check_spec),
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
        workload_id,
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

fn secret_provider(
    cluster_id: ClusterId,
    store: Arc<Mutex<RuntimeObjectStore>>,
    custody: Arc<Mutex<ClusterCustody>>,
    bindings: Arc<Mutex<BTreeMap<AttemptId, SecretBinding>>>,
) -> SecretPlanProvider {
    Arc::new(move |placement| {
        let bindings = bindings
            .lock()
            .map_err(|_| "secret binding lock poisoned".to_string())?;
        let binding = bindings
            .get(&placement.attempt_id)
            .ok_or("missing secret binding")?;
        let key = final_capsule_key(cluster_id, binding.capsule_id).map_err(|e| e.to_string())?;
        let store = store
            .lock()
            .map_err(|_| "object store lock poisoned".to_string())?;
        let meta =
            StreamingCapsuleReader::new(store.get_reader(&key, None).map_err(|e| e.to_string())?)
                .verify()
                .map_err(|e| e.to_string())?;
        let public = read_segment(&*store, &key, &meta, SegmentType::PublicMetadata)?;
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
        let release = ReleaseMetadataV1::decode(public.as_slice()).map_err(|e| e.to_string())?;
        let vars: BTreeMap<_, _> = release
            .runtime_variables
            .into_iter()
            .map(|v| (v.logical_name.clone(), v))
            .collect();
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
                },
                _ => return Err("unsupported secret injection kind".into()),
            };
            values.push(SecretValue {
                logical_name: value.logical_name,
                form,
                bytes: Secret::new(value.value),
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
    }
}

fn pipe_factory(plane: Arc<Mutex<TelemetryPlane>>) -> PipeSinkFactory {
    Arc::new(move |_| {
        Arc::new(TelemetrySink {
            plane: Arc::clone(&plane),
            stdout: AtomicU64::new(0),
            stderr: AtomicU64::new(0),
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

fn check_spec(spec: &CheckSpecV1) -> Option<CheckSpec> {
    let kind = match PbCheckKind::try_from(spec.kind).ok()? {
        PbCheckKind::Process => CheckKind::Process,
        PbCheckKind::Tcp => CheckKind::Tcp,
        PbCheckKind::Http => CheckKind::Http,
        PbCheckKind::Command => CheckKind::Command,
        _ => return None,
    };
    Some(CheckSpec {
        kind,
        target: spec.path.clone().or_else(|| spec.port_name.clone()),
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
    })
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
stable!(NodeId, UnitId, AttemptId, WorkloadId);

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
