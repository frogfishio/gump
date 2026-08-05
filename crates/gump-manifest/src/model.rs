//! Normalized `gump/1` manifest model.

use std::collections::BTreeMap;

use gump_types::Label;

/// Fully normalized, validated manifest (F01 product type).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Manifest {
    pub schema: SchemaVersion,
    pub app: App,
    pub workload: Workload,
    pub package: Package,
    pub prepare: Option<Prepare>,
    pub runtime: Runtime,
    pub health: Option<Health>,
    pub resources: Option<Resources>,
    pub deploy: Option<Deploy>,
    pub discovery: Option<Discovery>,
    pub publish: Option<Publish>,
    pub telemetry: Option<Telemetry>,
    pub local: Option<Local>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SchemaVersion {
    Gump1,
}

impl SchemaVersion {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gump1 => "gump/1",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct App {
    pub id: Label,
    pub namespace: Label,
    pub description: Option<String>,
    pub version: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Lifetime {
    Finite,
    Continuous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Coordination {
    Independent,
    Gang,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SuccessPolicy {
    Never,
    AnyExitZero,
    AllExitZero,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum FailurePolicy {
    FailUnit,
    RestartUnit,
    FailGroup,
    RestartGroup,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum IsolationPolicy {
    ContinueExisting,
    StopOnIsolation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Workload {
    pub lifetime: Lifetime,
    pub coordination: Coordination,
    pub success: SuccessPolicy,
    pub failure: Option<FailurePolicy>,
    pub max_attempts: Option<u32>,
    pub isolation: Option<IsolationPolicy>,
    /// Normalized milliseconds.
    pub isolation_grace_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Package {
    pub root: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub format: PackageFormat,
    pub allow_workspace_root: bool,
    pub allow_sensitive_files: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum PackageFormat {
    TarZstd,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prepare {
    pub command: Vec<String>,
    pub outputs: Vec<PrepareOutput>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareOutput {
    pub from: String,
    pub to: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Driver {
    Native,
    Script,
    Oci,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum StopSignal {
    Term,
    Int,
    Quit,
    Hup,
    Usr1,
    Usr2,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Runtime {
    pub driver: Driver,
    pub command: Vec<String>,
    pub interpreter: Option<Vec<String>>,
    pub workdir: Option<String>,
    pub stop_signal: Option<StopSignal>,
    pub stop_timeout_ms: Option<u64>,
    pub variables: BTreeMap<String, Variable>,
    pub ports: BTreeMap<Label, Port>,
    pub isolation: Option<RuntimeIsolation>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Classification {
    Internal,
    Secret,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Encoding {
    Utf8,
    Bytes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Inject {
    Env,
    Fd,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Variable {
    pub source: String,
    pub required: bool,
    pub classification: Classification,
    pub encoding: Option<Encoding>,
    pub max_bytes: Option<u64>,
    pub inject: Inject,
    pub fd: Option<u16>,
    pub reference_env: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortValue {
    Auto,
    Fixed(u16),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Port {
    pub address: String,
    pub value: PortValue,
    pub inject: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum IsolationProfile {
    None,
    Observed,
    Sandboxed,
    Strict,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeIsolation {
    pub profile: Option<IsolationProfile>,
    pub core_dumps: Option<AllowDeny>,
    pub swap_secrets: Option<AllowDeny>,
    pub proc_visibility: Option<ProcVisibility>,
    pub ptrace: Option<AllowDeny>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum AllowDeny {
    Allow,
    Deny,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ProcVisibility {
    Host,
    Restricted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CheckType {
    Process,
    Tcp,
    Http,
    Command,
    Fd,
    External,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Check {
    pub check_type: CheckType,
    pub port: Option<Label>,
    pub path: Option<String>,
    pub command: Option<Vec<String>>,
    pub interval_ms: u64,
    pub timeout_ms: u64,
    pub initial_delay_ms: Option<u64>,
    pub successes: Option<u32>,
    pub failures: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Health {
    pub readiness: Option<Check>,
    pub liveness: Option<Check>,
    pub progress: Option<Check>,
    pub completion: Option<Check>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Resources {
    pub cpu_request: Option<String>,
    pub cpu_limit: Option<String>,
    pub memory_request: Option<u64>,
    pub memory_limit: Option<u64>,
    pub ephemeral_request: Option<u64>,
    pub ephemeral_limit: Option<u64>,
    pub gpu_count: Option<u32>,
    pub gpu_vendor: Option<String>,
    pub gpu_model: Option<String>,
    pub gpu_memory_min: Option<u64>,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Coverage {
    Fixed,
    AllNodes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Priority {
    Low,
    Normal,
    High,
    Critical,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Deploy {
    pub units: Option<u32>,
    pub coverage: Option<Coverage>,
    pub priority: Option<Priority>,
    pub preemptible: Option<bool>,
    pub rollout: Option<Rollout>,
    pub placement: Option<Placement>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum RolloutStrategy {
    Replace,
    Rolling,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rollout {
    pub strategy: Option<RolloutStrategy>,
    pub max_unavailable: Option<u32>,
    pub max_surge: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Placement {
    pub spread: Vec<String>,
    pub require: Vec<String>,
    pub prefer: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Discovery {
    pub hiccup: Option<HiccupDiscovery>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum HealthBinding {
    Readiness,
    Liveness,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HiccupDiscovery {
    pub required_for_eligibility: Option<bool>,
    pub health_binding: Option<HealthBinding>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum PublishProtocol {
    Tcp,
    Http,
    Https,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Publish {
    pub provider: Label,
    pub required: bool,
    pub service: Label,
    pub port: Label,
    pub domain: Option<String>,
    pub protocol: Option<PublishProtocol>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Telemetry {
    pub protocol: String,
    pub format: String,
    pub filter: Option<String>,
    pub relay: Option<TelemetryRelay>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Overflow {
    DropOldest,
    DropNewest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TelemetryRelay {
    pub capacity: Option<u64>,
    pub max_record: Option<u64>,
    pub overflow: Option<Overflow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Local {
    pub watch: Vec<String>,
    pub ports: BTreeMap<String, u16>,
    pub variables: BTreeMap<String, LocalVariable>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalVariable {
    pub source: String,
}
