//! Developer-process packaging of public variable contracts + protected values
//! (FORMATS §5–§7 / F05 / GUMP-N007).
//!
//! Values are resolved only here (env / local override). Public metadata carries
//! names and contracts; ciphertext alone carries value bytes.

use std::collections::BTreeMap;

use gump_manifest::{
    AllowDeny, Check, CheckType, Classification, Coordination, Coverage, Driver, Encoding,
    FailurePolicy, HealthBinding, Inject, IsolationPolicy, IsolationProfile, Lifetime, Manifest,
    Overflow, PortValue, Priority, ProcVisibility, PublishProtocol, RolloutStrategy, StopSignal,
    SuccessPolicy, Variable,
};
use gump_protocol::pb;
use gump_protocol::pb::{
    AppIdentityV1, InjectionKind, ProtectedConfigV1, ProtectedValueV1, ReleaseMetadataV1,
    RuntimeVariableV1, ValueClassification, ValueEncoding,
};
use gump_types::{CapsuleId, ClusterId, Secret};
use prost::Message;

use crate::error::{CliError, CliErrorKind};

/// Packaged Capsule configuration segments ready for AEAD + signing.
#[derive(Debug)]
pub struct PackagedConfig {
    /// Canonical `ReleaseMetadataV1` protobuf (no protected values).
    pub public_metadata: Vec<u8>,
    /// Canonical `ProtectedConfigV1` protobuf plaintext (Secret-wrapped).
    pub protected_plaintext: Secret<Vec<u8>>,
}

/// Resolve manifest variables and encode public + protected packaging records.
///
/// Fail-closed before Capsule publish: required unset, oversize, malformed
/// source, unknown/unsupported provider schemes.
pub fn package_release_config(
    manifest: &Manifest,
    capsule_id: CapsuleId,
    cluster_id: ClusterId,
) -> Result<PackagedConfig, CliError> {
    package_release_config_with_env(manifest, capsule_id, cluster_id, &BTreeMap::new())
}

/// Same as [`package_release_config`], with optional in-process env overrides
/// (tests / controlled packaging without mutating process environment).
pub fn package_release_config_with_env(
    manifest: &Manifest,
    capsule_id: CapsuleId,
    cluster_id: ClusterId,
    env_overrides: &BTreeMap<String, Vec<u8>>,
) -> Result<PackagedConfig, CliError> {
    let mut runtime_variables = Vec::new();
    let mut protected_values = Vec::new();

    for (name, var) in &manifest.runtime.variables {
        runtime_variables.push(to_runtime_variable(name, var)?);
        if let Some(value) = resolve_variable(manifest, name, var, env_overrides)? {
            let bytes = value.expose().clone();
            protected_values.push(ProtectedValueV1 {
                logical_name: name.clone(),
                classification: classification_wire(var.classification) as i32,
                encoding: encoding_wire(var.encoding) as i32,
                injection: injection_wire(var.inject) as i32,
                present: true,
                value: bytes,
            });
        } else {
            // Optional unset: record absence with empty bytes (FORMATS §7).
            protected_values.push(ProtectedValueV1 {
                logical_name: name.clone(),
                classification: classification_wire(var.classification) as i32,
                encoding: encoding_wire(var.encoding) as i32,
                injection: injection_wire(var.inject) as i32,
                present: false,
                value: Vec::new(),
            });
        }
    }

    // BTreeMap iteration is already sorted; keep explicit sort for protobuf contract.
    runtime_variables.sort_by(|a, b| a.logical_name.cmp(&b.logical_name));
    protected_values.sort_by(|a, b| a.logical_name.cmp(&b.logical_name));

    let public = ReleaseMetadataV1 {
        schema: "gump.release/1".to_string(),
        capsule_id: capsule_id.as_bytes().to_vec(),
        cluster_id: cluster_id.as_bytes().to_vec(),
        app: Some(AppIdentityV1 {
            namespace: manifest.app.namespace.to_string(),
            app_id: manifest.app.id.to_string(),
            workload_id: None,
            description: None,
            version_annotation: None,
        }),
        normalized_manifest: Some(to_manifest_v1(manifest, runtime_variables.clone())?),
        archive: None,
        build: None,
        required_capabilities: Vec::new(),
        runtime_variables,
    };

    let protected = ProtectedConfigV1 {
        schema: "gump.protected/1".to_string(),
        capsule_id: capsule_id.as_bytes().to_vec(),
        cluster_id: cluster_id.as_bytes().to_vec(),
        values: protected_values,
    };

    let mut public_metadata = Vec::with_capacity(public.encoded_len());
    public.encode(&mut public_metadata).map_err(|e| {
        CliError::new(
            CliErrorKind::Capsule,
            format!("encode release metadata: {e}"),
        )
    })?;

    let mut protected_bytes = Vec::with_capacity(protected.encoded_len());
    protected.encode(&mut protected_bytes).map_err(|e| {
        CliError::new(
            CliErrorKind::Crypto,
            format!("encode protected config: {e}"),
        )
    })?;

    Ok(PackagedConfig {
        public_metadata,
        protected_plaintext: Secret::new(protected_bytes),
    })
}

fn to_manifest_v1(
    manifest: &Manifest,
    variables: Vec<RuntimeVariableV1>,
) -> Result<pb::ManifestV1, CliError> {
    let app = AppIdentityV1 {
        namespace: manifest.app.namespace.to_string(),
        app_id: manifest.app.id.to_string(),
        workload_id: Some(stable_workload_id(manifest).to_vec()),
        description: manifest.app.description.clone(),
        version_annotation: manifest.app.version.clone(),
    };
    let workload = pb::WorkloadSpecV1 {
        lifetime: match manifest.workload.lifetime {
            Lifetime::Finite => pb::WorkloadLifetime::Finite,
            Lifetime::Continuous => pb::WorkloadLifetime::Continuous,
        } as i32,
        coordination: match manifest.workload.coordination {
            Coordination::Independent => pb::CoordinationKind::Independent,
            Coordination::Gang => pb::CoordinationKind::Gang,
        } as i32,
        success: match manifest.workload.success {
            SuccessPolicy::Never => pb::SuccessKind::Never,
            SuccessPolicy::AnyExitZero => pb::SuccessKind::AnyExitZero,
            SuccessPolicy::AllExitZero => pb::SuccessKind::AllExitZero,
        } as i32,
        failure: match manifest.workload.failure.unwrap_or(FailurePolicy::FailUnit) {
            FailurePolicy::FailUnit => pb::FailureKind::FailUnit,
            FailurePolicy::RestartUnit => pb::FailureKind::RestartUnit,
            FailurePolicy::FailGroup => pb::FailureKind::FailGroup,
            FailurePolicy::RestartGroup => pb::FailureKind::RestartGroup,
        } as i32,
        max_attempts: manifest.workload.max_attempts,
        stop_on_isolation: matches!(
            manifest.workload.isolation,
            Some(IsolationPolicy::StopOnIsolation)
        ),
        isolation_grace_ms: manifest.workload.isolation_grace_ms.unwrap_or(0),
    };
    let mut ports = manifest
        .runtime
        .ports
        .iter()
        .map(|(name, port)| pb::PortSpecV1 {
            name: name.to_string(),
            address: port.address.clone(),
            fixed_port: match port.value {
                PortValue::Fixed(p) => Some(u32::from(p)),
                PortValue::Auto => None,
            },
            allocate_automatically: matches!(port.value, PortValue::Auto),
            inject_env: port.inject.clone(),
        })
        .collect::<Vec<_>>();
    ports.sort_by(|a, b| a.name.cmp(&b.name));
    let runtime = pb::RuntimeSpecV1 {
        driver: match manifest.runtime.driver {
            Driver::Native => pb::DriverKind::Native,
            Driver::Script => pb::DriverKind::Script,
            Driver::Oci => pb::DriverKind::Oci,
        } as i32,
        command: manifest.runtime.command.clone(),
        interpreter: manifest.runtime.interpreter.clone().unwrap_or_default(),
        workdir: manifest.runtime.workdir.clone().unwrap_or_default(),
        stop_signal: manifest
            .runtime
            .stop_signal
            .map(stop_signal_name)
            .unwrap_or("term")
            .into(),
        stop_timeout_ms: manifest.runtime.stop_timeout_ms.unwrap_or(10_000),
        variables,
        ports,
        isolation: manifest
            .runtime
            .isolation
            .as_ref()
            .map(|i| pb::IsolationSpecV1 {
                profile: match i.profile.unwrap_or(IsolationProfile::Observed) {
                    IsolationProfile::None => "none",
                    IsolationProfile::Observed => "observed",
                    IsolationProfile::Sandboxed => "sandboxed",
                    IsolationProfile::Strict => "strict",
                }
                .into(),
                deny_core_dumps: matches!(i.core_dumps, Some(AllowDeny::Deny)),
                deny_secret_swap: matches!(i.swap_secrets, Some(AllowDeny::Deny)),
                restrict_proc: matches!(i.proc_visibility, Some(ProcVisibility::Restricted)),
                deny_ptrace: matches!(i.ptrace, Some(AllowDeny::Deny)),
            }),
    };
    let health = manifest.health.as_ref().map(|h| pb::HealthSpecV1 {
        readiness: h.readiness.as_ref().map(check_v1),
        liveness: h.liveness.as_ref().map(check_v1),
        progress: h.progress.as_ref().map(check_v1),
        completion: h.completion.as_ref().map(check_v1),
    });
    let resources = manifest.resources.as_ref().map(|r| {
        let mut capabilities = r.capabilities.clone();
        capabilities.sort();
        pb::ResourceSpecV1 {
            cpu_request_millis: r.cpu_request.as_deref().and_then(cpu_millis),
            cpu_limit_millis: r.cpu_limit.as_deref().and_then(cpu_millis),
            memory_request_bytes: r.memory_request,
            memory_limit_bytes: r.memory_limit,
            ephemeral_request_bytes: r.ephemeral_request,
            ephemeral_limit_bytes: r.ephemeral_limit,
            gpu_count: r.gpu_count,
            gpu_vendor: r.gpu_vendor.clone(),
            gpu_model: r.gpu_model.clone(),
            gpu_memory_min_bytes: r.gpu_memory_min,
            capabilities,
        }
    });
    let deploy = manifest.deploy.as_ref().map(|d| pb::DeployDefaultsV1 {
        units: d.units,
        priority_request: match d.priority.unwrap_or(Priority::Normal) {
            Priority::Low => "low",
            Priority::Normal => "normal",
            Priority::High => "high",
            Priority::Critical => "critical",
        }
        .into(),
        preemptible_request: d.preemptible.unwrap_or(false),
        rollout: d.rollout.as_ref().map(|r| pb::RolloutSpecV1 {
            strategy: match r.strategy.unwrap_or(RolloutStrategy::Replace) {
                RolloutStrategy::Replace => "replace",
                RolloutStrategy::Rolling => "rolling",
            }
            .into(),
            max_unavailable: r.max_unavailable.unwrap_or(0),
            max_surge: r.max_surge.unwrap_or(0),
        }),
        placement: d.placement.as_ref().map(|p| pb::PlacementSpecV1 {
            spread: p.spread.clone(),
            require: p.require.clone(),
            prefer: p.prefer.clone(),
        }),
        coverage: match d.coverage.unwrap_or(Coverage::Fixed) {
            Coverage::Fixed => pb::CoverageKind::Fixed,
            Coverage::AllNodes => pb::CoverageKind::AllNodes,
        } as i32,
    });
    let publication = manifest.publish.as_ref().map(|p| pb::PublicationSpecV1 {
        provider: p.provider.to_string(),
        required: p.required,
        service: p.service.to_string(),
        port_name: p.port.to_string(),
        domain: p.domain.clone(),
        protocol: match p.protocol.unwrap_or(PublishProtocol::Tcp) {
            PublishProtocol::Tcp => "tcp",
            PublishProtocol::Http => "http",
            PublishProtocol::Https => "https",
        }
        .into(),
    });
    let telemetry = manifest.telemetry.as_ref().map(|t| pb::TelemetrySpecV1 {
        producer_protocol: t.protocol.clone(),
        filter: t.filter.clone().unwrap_or_default(),
        relay_capacity_bytes: t.relay.as_ref().and_then(|r| r.capacity).unwrap_or(0),
        max_record_bytes: t.relay.as_ref().and_then(|r| r.max_record).unwrap_or(0),
        overflow: match t.relay.as_ref().and_then(|r| r.overflow) {
            Some(Overflow::DropNewest) => "drop_newest",
            _ => "drop_oldest",
        }
        .into(),
    });
    let hiccup = manifest
        .discovery
        .as_ref()
        .and_then(|d| d.hiccup.as_ref())
        .map(|h| pb::HiccupDiscoverySpecV1 {
            required_for_eligibility: h.required_for_eligibility.unwrap_or(false),
            health_binding: h.health_binding.map(|b| match b {
                HealthBinding::Readiness => "readiness".into(),
                HealthBinding::Liveness => "liveness".into(),
            }),
        });
    Ok(pb::ManifestV1 {
        schema: manifest.schema.as_str().into(),
        app: Some(app),
        workload: Some(workload),
        runtime: Some(runtime),
        health,
        resources,
        deploy,
        publication,
        telemetry,
        hiccup,
    })
}

fn stable_workload_id(manifest: &Manifest) -> [u8; 16] {
    let mut input = Vec::new();
    input.extend_from_slice(manifest.app.namespace.to_string().as_bytes());
    input.push(0);
    input.extend_from_slice(manifest.app.id.to_string().as_bytes());
    let hash = blake3::hash(&input);
    let mut id = [0u8; 16];
    id.copy_from_slice(&hash.as_bytes()[..16]);
    id[6] = (id[6] & 0x0f) | 0x70;
    id[8] = (id[8] & 0x3f) | 0x80;
    id
}

fn check_v1(c: &Check) -> pb::CheckSpecV1 {
    pb::CheckSpecV1 {
        kind: match c.check_type {
            CheckType::Process => pb::CheckKind::Process,
            CheckType::Tcp => pb::CheckKind::Tcp,
            CheckType::Http => pb::CheckKind::Http,
            CheckType::Command => pb::CheckKind::Command,
            CheckType::Fd => pb::CheckKind::Fd,
            CheckType::External => pb::CheckKind::External,
        } as i32,
        port_name: c.port.as_ref().map(ToString::to_string),
        path: c.path.clone(),
        command: c.command.clone().unwrap_or_default(),
        interval_ms: c.interval_ms,
        timeout_ms: c.timeout_ms,
        initial_delay_ms: c.initial_delay_ms.unwrap_or(0),
        successes: c.successes.unwrap_or(1),
        failures: c.failures.unwrap_or(1),
    }
}

fn stop_signal_name(signal: StopSignal) -> &'static str {
    match signal {
        StopSignal::Term => "term",
        StopSignal::Int => "int",
        StopSignal::Quit => "quit",
        StopSignal::Hup => "hup",
        StopSignal::Usr1 => "usr1",
        StopSignal::Usr2 => "usr2",
    }
}

fn cpu_millis(raw: &str) -> Option<u64> {
    raw.strip_suffix('m')
        .and_then(|n| n.parse().ok())
        .or_else(|| {
            raw.parse::<u64>()
                .ok()
                .map(|cores| cores.saturating_mul(1_000))
        })
}

fn to_runtime_variable(name: &str, var: &Variable) -> Result<RuntimeVariableV1, CliError> {
    Ok(RuntimeVariableV1 {
        logical_name: name.to_string(),
        required: var.required,
        classification: classification_wire(var.classification) as i32,
        encoding: encoding_wire(var.encoding) as i32,
        max_bytes: var.max_bytes.unwrap_or(0),
        injection: injection_wire(var.inject) as i32,
        inherited_fd: var.fd.map(u32::from),
        reference_env: var.reference_env.clone(),
    })
}

fn resolve_variable(
    manifest: &Manifest,
    name: &str,
    var: &Variable,
    env_overrides: &BTreeMap<String, Vec<u8>>,
) -> Result<Option<Secret<Vec<u8>>>, CliError> {
    let source = manifest
        .local
        .as_ref()
        .and_then(|local| local.variables.get(name))
        .map(|lv| lv.source.as_str())
        .unwrap_or(var.source.as_str());

    let raw = match read_source(source, env_overrides)? {
        Some(v) => v,
        None => {
            if var.required {
                return Err(CliError::new(
                    CliErrorKind::Policy,
                    format!("required variable {name:?} unset (source scheme redacted)"),
                ));
            }
            return Ok(None);
        }
    };

    if let Some(max) = var.max_bytes {
        if raw.expose().len() as u64 > max {
            return Err(CliError::new(
                CliErrorKind::Policy,
                format!(
                    "variable {name:?} exceeds max_bytes bound ({max}; got {} bytes)",
                    raw.expose().len()
                ),
            ));
        }
    }

    if matches!(var.encoding, Some(Encoding::Utf8) | None)
        && std::str::from_utf8(raw.expose()).is_err()
    {
        return Err(CliError::new(
            CliErrorKind::Policy,
            format!("variable {name:?} is not valid UTF-8"),
        ));
    }

    Ok(Some(raw))
}

fn read_source(
    source: &str,
    env_overrides: &BTreeMap<String, Vec<u8>>,
) -> Result<Option<Secret<Vec<u8>>>, CliError> {
    if let Some(env_name) = source.strip_prefix("env:") {
        if env_name.is_empty() || env_name.bytes().any(|b| b == 0) {
            return Err(CliError::new(
                CliErrorKind::Policy,
                "malformed env variable source",
            ));
        }
        if let Some(bytes) = env_overrides.get(env_name) {
            return Ok(Some(Secret::new(bytes.clone())));
        }
        match std::env::var_os(env_name) {
            Some(os) => {
                let bytes = os_to_bytes(os)?;
                Ok(Some(Secret::new(bytes)))
            }
            None => Ok(None),
        }
    } else if let Some(literal) = source.strip_prefix("literal:") {
        // Test / local-dev only; never for production secret providers.
        Ok(Some(Secret::new(literal.as_bytes().to_vec())))
    } else if source.starts_with("provider:") {
        Err(CliError::new(
            CliErrorKind::Policy,
            "variable provider error: provider scheme not configured",
        ))
    } else {
        Err(CliError::new(
            CliErrorKind::Policy,
            "malformed variable source (unsupported scheme)",
        ))
    }
}

fn os_to_bytes(os: std::ffi::OsString) -> Result<Vec<u8>, CliError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(os.into_vec())
    }
    #[cfg(not(unix))]
    {
        os.into_string()
            .map(|s| s.into_bytes())
            .map_err(|_| CliError::new(CliErrorKind::Policy, "variable value is not valid Unicode"))
    }
}

fn classification_wire(c: Classification) -> ValueClassification {
    match c {
        Classification::Internal => ValueClassification::Internal,
        Classification::Secret => ValueClassification::Secret,
    }
}

fn encoding_wire(e: Option<Encoding>) -> ValueEncoding {
    match e {
        Some(Encoding::Utf8) | None => ValueEncoding::Utf8,
        Some(Encoding::Bytes) => ValueEncoding::Bytes,
    }
}

fn injection_wire(i: Inject) -> InjectionKind {
    match i {
        Inject::Env => InjectionKind::Env,
        Inject::Fd => InjectionKind::Fd,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gump_manifest::parse_manifest_str;

    fn manifest_with_greeting() -> Manifest {
        parse_manifest_str(
            r#"
schema = "gump/1"
[app]
id = "pack-test"
namespace = "ci"
[workload]
lifetime = "finite"
coordination = "independent"
success = "all_exit_zero"
[package]
root = "."
include = ["bin/hello"]
[runtime]
driver = "native"
command = ["./bin/hello"]
[runtime.variables.GREETING]
source = "env:GUMP_N007_GREETING"
required = true
classification = "internal"
encoding = "utf8"
max_bytes = "4KiB"
inject = "env"
[runtime.variables.TOKEN]
source = "env:GUMP_N007_TOKEN"
required = true
classification = "secret"
encoding = "utf8"
max_bytes = "4KiB"
inject = "env"
[telemetry]
protocol = "ratatouille/0.1"
format = "ndjson"
"#,
        )
        .expect("manifest")
    }

    fn fixed_v7(tag: u8) -> [u8; 16] {
        let mut b = [
            0x01, 0x8f, 0x4a, 0x00, 0x00, 0x00, 0x70, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        b[15] = tag;
        b
    }

    #[test]
    fn packages_contracts_without_secret_bytes_in_public() {
        let canary = "n007-canary-SECRET-value-9f3a";
        let m = manifest_with_greeting();
        let cap = CapsuleId::from_bytes(fixed_v7(0x11)).unwrap();
        let clu = ClusterId::from_bytes(fixed_v7(0x12)).unwrap();
        let mut env = BTreeMap::new();
        env.insert("GUMP_N007_GREETING".to_string(), b"hello".to_vec());
        env.insert("GUMP_N007_TOKEN".to_string(), canary.as_bytes().to_vec());
        let packed = package_release_config_with_env(&m, cap, clu, &env).unwrap();

        assert!(
            !packed
                .public_metadata
                .windows(canary.len())
                .any(|w| w == canary.as_bytes())
        );
        assert!(!String::from_utf8_lossy(&packed.public_metadata).contains(canary));

        let release = ReleaseMetadataV1::decode(packed.public_metadata.as_slice()).unwrap();
        assert_eq!(release.schema, "gump.release/1");
        assert_eq!(release.runtime_variables.len(), 2);
        assert!(
            release
                .runtime_variables
                .iter()
                .any(|v| v.logical_name == "TOKEN"
                    && v.classification == ValueClassification::Secret as i32)
        );

        let protected =
            ProtectedConfigV1::decode(packed.protected_plaintext.expose().as_slice()).unwrap();
        assert_eq!(protected.schema, "gump.protected/1");
        let token = protected
            .values
            .iter()
            .find(|v| v.logical_name == "TOKEN")
            .unwrap();
        assert_eq!(token.value, canary.as_bytes());
        assert!(token.present);
    }

    #[test]
    fn required_unset_fails_closed() {
        let m = manifest_with_greeting();
        let err = package_release_config(
            &m,
            CapsuleId::from_bytes(fixed_v7(0x13)).unwrap(),
            ClusterId::from_bytes(fixed_v7(0x14)).unwrap(),
        )
        .unwrap_err();
        assert_eq!(err.kind(), CliErrorKind::Policy);
        let msg = err.to_string();
        assert!(!msg.contains("n007-canary"));
        assert!(msg.contains("unset") || msg.contains("required"));
    }

    #[test]
    fn provider_scheme_fails_before_publish() {
        let m = parse_manifest_str(
            r#"
schema = "gump/1"
[app]
id = "pack-test"
namespace = "ci"
[workload]
lifetime = "finite"
coordination = "independent"
success = "all_exit_zero"
[package]
root = "."
include = ["bin/hello"]
[runtime]
driver = "native"
command = ["./bin/hello"]
[runtime.variables.X]
source = "provider:vault/path"
required = true
classification = "secret"
inject = "env"
[telemetry]
protocol = "ratatouille/0.1"
format = "ndjson"
"#,
        )
        .unwrap();
        let err = package_release_config(
            &m,
            CapsuleId::from_bytes(fixed_v7(0x15)).unwrap(),
            ClusterId::from_bytes(fixed_v7(0x16)).unwrap(),
        )
        .unwrap_err();
        assert_eq!(err.kind(), CliErrorKind::Policy);
        assert!(err.to_string().contains("provider"));
    }
}
