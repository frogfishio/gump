//! TOML deserialize surface (`deny_unknown_fields` everywhere).

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::error::{ManifestError, ManifestErrorKind};
use crate::model::Manifest;
use crate::normalize::normalize_manifest;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawManifest {
    pub schema: String,
    pub app: RawApp,
    pub workload: RawWorkload,
    pub package: RawPackage,
    #[serde(default)]
    pub prepare: Option<RawPrepare>,
    pub runtime: RawRuntime,
    #[serde(default)]
    pub health: Option<RawHealth>,
    #[serde(default)]
    pub resources: Option<RawResources>,
    #[serde(default)]
    pub deploy: Option<RawDeploy>,
    #[serde(default)]
    pub discovery: Option<RawDiscovery>,
    #[serde(default)]
    pub provides: BTreeMap<String, RawProvidedCapability>,
    #[serde(default)]
    pub publish: Option<RawPublish>,
    #[serde(default)]
    pub telemetry: Option<RawTelemetry>,
    #[serde(default)]
    pub local: Option<RawLocal>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawProvidedCapability {
    pub protocol: String,
    pub port: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    pub authentication: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawApp {
    pub id: String,
    pub namespace: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawWorkload {
    pub lifetime: String,
    pub coordination: String,
    pub success: String,
    #[serde(default)]
    pub failure: Option<String>,
    #[serde(default)]
    pub max_attempts: Option<u32>,
    #[serde(default)]
    pub isolation: Option<String>,
    #[serde(default)]
    pub isolation_grace: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawPackage {
    pub root: String,
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub allow_workspace_root: Option<bool>,
    #[serde(default)]
    pub allow_sensitive_files: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawPrepare {
    pub command: Vec<String>,
    pub outputs: Vec<RawPrepareOutput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawPrepareOutput {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawRuntime {
    pub driver: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub interpreter: Option<Vec<String>>,
    #[serde(default)]
    pub workdir: Option<String>,
    #[serde(default)]
    pub stop_signal: Option<String>,
    #[serde(default)]
    pub stop_timeout: Option<String>,
    #[serde(default)]
    pub variables: BTreeMap<String, RawVariable>,
    #[serde(default)]
    pub ports: BTreeMap<String, RawPort>,
    #[serde(default)]
    pub isolation: Option<RawRuntimeIsolation>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawVariable {
    pub source: String,
    pub required: bool,
    pub classification: String,
    #[serde(default)]
    pub encoding: Option<String>,
    #[serde(default)]
    pub max_bytes: Option<String>,
    pub inject: String,
    #[serde(default)]
    pub fd: Option<u16>,
    #[serde(default)]
    pub reference_env: Option<String>,
    #[serde(default)]
    pub reference_value: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawPort {
    pub address: String,
    pub value: RawPortValue,
    #[serde(default)]
    pub inject: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum RawPortValue {
    String(String),
    Int(i64),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawRuntimeIsolation {
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub core_dumps: Option<String>,
    #[serde(default)]
    pub swap_secrets: Option<String>,
    #[serde(default)]
    pub proc_visibility: Option<String>,
    #[serde(default)]
    pub ptrace: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawCheck {
    #[serde(rename = "type")]
    pub check_type: String,
    #[serde(default)]
    pub port: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub command: Option<Vec<String>>,
    pub interval: String,
    pub timeout: String,
    #[serde(default)]
    pub initial_delay: Option<String>,
    #[serde(default)]
    pub successes: Option<u32>,
    #[serde(default)]
    pub failures: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawHealth {
    #[serde(default)]
    pub readiness: Option<RawCheck>,
    #[serde(default)]
    pub liveness: Option<RawCheck>,
    #[serde(default)]
    pub progress: Option<RawCheck>,
    #[serde(default)]
    pub completion: Option<RawCheck>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawResources {
    #[serde(default)]
    pub cpu_request: Option<String>,
    #[serde(default)]
    pub cpu_limit: Option<String>,
    #[serde(default)]
    pub memory_request: Option<String>,
    #[serde(default)]
    pub memory_limit: Option<String>,
    #[serde(default)]
    pub ephemeral_request: Option<String>,
    #[serde(default)]
    pub ephemeral_limit: Option<String>,
    #[serde(default)]
    pub gpu_count: Option<u32>,
    #[serde(default)]
    pub gpu_vendor: Option<String>,
    #[serde(default)]
    pub gpu_model: Option<String>,
    #[serde(default)]
    pub gpu_memory_min: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawDeploy {
    #[serde(default)]
    pub units: Option<u32>,
    #[serde(default)]
    pub coverage: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub preemptible: Option<bool>,
    #[serde(default)]
    pub rollout: Option<RawRollout>,
    #[serde(default)]
    pub placement: Option<RawPlacement>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawRollout {
    #[serde(default)]
    pub strategy: Option<String>,
    #[serde(default)]
    pub max_unavailable: Option<u32>,
    #[serde(default)]
    pub max_surge: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawPlacement {
    #[serde(default)]
    pub spread: Vec<String>,
    #[serde(default)]
    pub require: Vec<String>,
    #[serde(default)]
    pub prefer: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawDiscovery {
    #[serde(default)]
    pub hiccup: Option<RawHiccup>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawHiccup {
    #[serde(default)]
    pub required_for_eligibility: Option<bool>,
    #[serde(default)]
    pub health_binding: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawPublish {
    pub provider: String,
    pub required: bool,
    pub service: String,
    pub port: String,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub protocol: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawTelemetry {
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub relay: Option<RawRelay>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawRelay {
    #[serde(default)]
    pub capacity: Option<String>,
    #[serde(default)]
    pub max_record: Option<String>,
    #[serde(default)]
    pub overflow: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawLocal {
    #[serde(default)]
    pub watch: Vec<String>,
    #[serde(default)]
    pub ports: BTreeMap<String, u16>,
    #[serde(default)]
    pub variables: BTreeMap<String, RawLocalVariable>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawLocalVariable {
    pub source: String,
}

/// Parse a `gump.toml` document into a normalized [`Manifest`].
pub fn parse_manifest_str(input: &str) -> Result<Manifest, ManifestError> {
    let raw: RawManifest = toml::from_str(input).map_err(|e| {
        let msg = e.message().to_string();
        let kind = if msg.contains("unknown field") {
            ManifestErrorKind::UnknownKey
        } else if msg.contains("missing field") {
            ManifestErrorKind::MissingField
        } else {
            ManifestErrorKind::Toml
        };
        ManifestError::new(kind, "manifest", msg)
    })?;
    normalize_manifest(raw)
}

/// Parse from an already-decoded TOML value (tests / tooling).
pub fn parse_manifest_value(value: toml::Value) -> Result<Manifest, ManifestError> {
    let raw: RawManifest = value.try_into().map_err(|e: toml::de::Error| {
        let msg = e.message().to_string();
        let kind = if msg.contains("unknown field") {
            ManifestErrorKind::UnknownKey
        } else if msg.contains("missing field") {
            ManifestErrorKind::MissingField
        } else {
            ManifestErrorKind::Toml
        };
        ManifestError::new(kind, "manifest", msg)
    })?;
    normalize_manifest(raw)
}
