//! Raw → normalized conversion + semantic checks.

use std::collections::BTreeMap;

use gump_types::{Label, LabelError};

use crate::error::{ManifestError, ManifestErrorKind};
use crate::model::*;
use crate::parse::*;
use crate::scalar::{parse_byte_size, parse_duration_millis};

pub(crate) fn normalize_manifest(raw: RawManifest) -> Result<Manifest, ManifestError> {
    if raw.schema != "gump/1" {
        return Err(ManifestError::new(
            ManifestErrorKind::Schema,
            "schema",
            format!("unsupported schema {:?}", raw.schema),
        ));
    }

    let app = normalize_app(raw.app)?;
    let workload = normalize_workload(raw.workload)?;
    let package = normalize_package(raw.package)?;
    let prepare = raw.prepare.map(normalize_prepare).transpose()?;
    let runtime = normalize_runtime(raw.runtime)?;
    let health = raw.health.map(normalize_health).transpose()?;
    let resources = raw.resources.map(normalize_resources).transpose()?;
    let deploy = raw.deploy.map(normalize_deploy).transpose()?;
    let discovery = raw.discovery.map(normalize_discovery).transpose()?;
    let mut provides = BTreeMap::new();
    for (name, capability) in raw.provides {
        if name.is_empty()
            || name.len() > 63
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(ManifestError::new(
                ManifestErrorKind::InvalidValue,
                format!("provides.{name}"),
                "capability name must contain only lowercase ASCII, digits, and underscore",
            ));
        }
        let port = label(&format!("provides.{name}.port"), &capability.port)?;
        if !runtime.ports.contains_key(&port) {
            return Err(ManifestError::new(
                ManifestErrorKind::Semantic,
                format!("provides.{name}.port"),
                "provided capability refers to an undeclared runtime port",
            ));
        }
        if capability.protocol.is_empty() || capability.authentication.is_empty() {
            return Err(ManifestError::new(
                ManifestErrorKind::InvalidValue,
                format!("provides.{name}"),
                "protocol and authentication must be non-empty",
            ));
        }
        provides.insert(
            name,
            ProvidedCapability {
                protocol: capability.protocol,
                port,
                path: capability.path,
                scope: capability.scope,
                authentication: capability.authentication,
            },
        );
    }
    let publish = raw.publish.map(normalize_publish).transpose()?;
    let telemetry = raw.telemetry.map(normalize_telemetry).transpose()?;
    let local = raw.local.map(normalize_local).transpose()?;

    Ok(Manifest {
        schema: SchemaVersion::Gump1,
        app,
        workload,
        package,
        prepare,
        runtime,
        health,
        resources,
        deploy,
        discovery,
        provides,
        publish,
        telemetry,
        local,
    })
}

fn label(path: &str, raw: &str) -> Result<Label, ManifestError> {
    Label::parse(raw).map_err(|e: LabelError| {
        ManifestError::new(
            ManifestErrorKind::InvalidValue,
            path,
            format!("invalid label {raw:?}: {e}"),
        )
    })
}

fn normalize_app(raw: RawApp) -> Result<App, ManifestError> {
    Ok(App {
        id: label("app.id", &raw.id)?,
        namespace: label("app.namespace", &raw.namespace)?,
        description: raw.description,
        version: raw.version,
    })
}

fn normalize_workload(raw: RawWorkload) -> Result<Workload, ManifestError> {
    Ok(Workload {
        lifetime: match raw.lifetime.as_str() {
            "finite" => Lifetime::Finite,
            "continuous" => Lifetime::Continuous,
            other => {
                return Err(invalid("workload.lifetime", other));
            }
        },
        coordination: match raw.coordination.as_str() {
            "independent" => Coordination::Independent,
            "gang" => Coordination::Gang,
            other => return Err(invalid("workload.coordination", other)),
        },
        success: match raw.success.as_str() {
            "never" => SuccessPolicy::Never,
            "any_exit_zero" => SuccessPolicy::AnyExitZero,
            "all_exit_zero" => SuccessPolicy::AllExitZero,
            other => return Err(invalid("workload.success", other)),
        },
        failure: raw
            .failure
            .as_deref()
            .map(|v| match v {
                "fail_unit" => Ok(FailurePolicy::FailUnit),
                "restart_unit" => Ok(FailurePolicy::RestartUnit),
                "fail_group" => Ok(FailurePolicy::FailGroup),
                "restart_group" => Ok(FailurePolicy::RestartGroup),
                other => Err(invalid("workload.failure", other)),
            })
            .transpose()?,
        max_attempts: raw.max_attempts,
        isolation: raw
            .isolation
            .as_deref()
            .map(|v| match v {
                "continue_existing" => Ok(IsolationPolicy::ContinueExisting),
                "stop_on_isolation" => Ok(IsolationPolicy::StopOnIsolation),
                other => Err(invalid("workload.isolation", other)),
            })
            .transpose()?,
        isolation_grace_ms: raw
            .isolation_grace
            .as_deref()
            .map(|d| {
                parse_duration_millis(d).map_err(|e| {
                    ManifestError::new(e.kind(), "workload.isolation_grace", e.message())
                })
            })
            .transpose()?,
    })
}

fn normalize_package(raw: RawPackage) -> Result<Package, ManifestError> {
    if raw.root.is_empty() {
        return Err(ManifestError::new(
            ManifestErrorKind::InvalidValue,
            "package.root",
            "root must be non-empty",
        ));
    }
    if raw.include.is_empty() {
        return Err(ManifestError::new(
            ManifestErrorKind::InvalidValue,
            "package.include",
            "include must be non-empty",
        ));
    }
    let format = match raw.format.as_deref().unwrap_or("tar+zstd") {
        "tar+zstd" => PackageFormat::TarZstd,
        other => return Err(invalid("package.format", other)),
    };
    Ok(Package {
        root: raw.root,
        include: raw.include,
        exclude: raw.exclude,
        format,
        allow_workspace_root: raw.allow_workspace_root.unwrap_or(false),
        allow_sensitive_files: raw.allow_sensitive_files.unwrap_or(false),
    })
}

fn normalize_prepare(raw: RawPrepare) -> Result<Prepare, ManifestError> {
    if raw.command.is_empty() {
        return Err(ManifestError::new(
            ManifestErrorKind::InvalidValue,
            "prepare.command",
            "command must be non-empty",
        ));
    }
    if raw.outputs.is_empty() {
        return Err(ManifestError::new(
            ManifestErrorKind::InvalidValue,
            "prepare.outputs",
            "outputs must be non-empty",
        ));
    }
    Ok(Prepare {
        command: raw.command,
        outputs: raw
            .outputs
            .into_iter()
            .map(|o| PrepareOutput {
                from: o.from,
                to: o.to,
            })
            .collect(),
    })
}

fn normalize_runtime(raw: RawRuntime) -> Result<Runtime, ManifestError> {
    if raw.command.is_empty() {
        return Err(ManifestError::new(
            ManifestErrorKind::InvalidValue,
            "runtime.command",
            "command must be non-empty",
        ));
    }
    let mut variables = BTreeMap::new();
    for (name, var) in raw.variables {
        validate_env_name("runtime.variables", &name)?;
        variables.insert(name, normalize_variable(var)?);
    }
    let mut ports = BTreeMap::new();
    for (name, port) in raw.ports {
        let key = label(&format!("runtime.ports.{name}"), &name)?;
        ports.insert(key, normalize_port(port)?);
    }
    Ok(Runtime {
        driver: match raw.driver.as_str() {
            "native" => Driver::Native,
            "script" => Driver::Script,
            "oci" => Driver::Oci,
            other => return Err(invalid("runtime.driver", other)),
        },
        command: raw.command,
        interpreter: raw.interpreter,
        workdir: raw.workdir,
        stop_signal: raw
            .stop_signal
            .as_deref()
            .map(|s| match s {
                "TERM" => Ok(StopSignal::Term),
                "INT" => Ok(StopSignal::Int),
                "QUIT" => Ok(StopSignal::Quit),
                "HUP" => Ok(StopSignal::Hup),
                "USR1" => Ok(StopSignal::Usr1),
                "USR2" => Ok(StopSignal::Usr2),
                other => Err(invalid("runtime.stop_signal", other)),
            })
            .transpose()?,
        stop_timeout_ms: raw
            .stop_timeout
            .as_deref()
            .map(|d| {
                parse_duration_millis(d)
                    .map_err(|e| ManifestError::new(e.kind(), "runtime.stop_timeout", e.message()))
            })
            .transpose()?,
        variables,
        ports,
        isolation: raw.isolation.map(normalize_runtime_isolation).transpose()?,
    })
}

fn normalize_variable(raw: RawVariable) -> Result<Variable, ManifestError> {
    let inject = match raw.inject.as_str() {
        "env" => Inject::Env,
        "fd" => Inject::Fd,
        other => return Err(invalid("runtime.variables.inject", other)),
    };
    if inject == Inject::Fd && raw.fd.is_none() {
        return Err(ManifestError::new(
            ManifestErrorKind::Semantic,
            "runtime.variables.fd",
            "fd inject requires fd",
        ));
    }
    Ok(Variable {
        source: raw.source,
        required: raw.required,
        classification: match raw.classification.as_str() {
            "internal" => Classification::Internal,
            "secret" => Classification::Secret,
            other => return Err(invalid("runtime.variables.classification", other)),
        },
        encoding: raw
            .encoding
            .as_deref()
            .map(|e| match e {
                "utf8" => Ok(Encoding::Utf8),
                "bytes" => Ok(Encoding::Bytes),
                other => Err(invalid("runtime.variables.encoding", other)),
            })
            .transpose()?,
        max_bytes: raw
            .max_bytes
            .as_deref()
            .map(|b| {
                parse_byte_size(b).map_err(|e| {
                    ManifestError::new(e.kind(), "runtime.variables.max_bytes", e.message())
                })
            })
            .transpose()?,
        inject,
        fd: raw.fd,
        reference_env: raw.reference_env,
        reference_value: match raw.reference_value.as_deref().unwrap_or("proc_path") {
            "proc_path" => FdReference::ProcPath,
            "descriptor_number" => FdReference::DescriptorNumber,
            other => return Err(invalid("runtime.variables.reference_value", other)),
        },
    })
}

fn normalize_port(raw: RawPort) -> Result<Port, ManifestError> {
    let value = match raw.value {
        RawPortValue::String(s) if s == "auto" => PortValue::Auto,
        RawPortValue::String(s) => {
            return Err(invalid("runtime.ports.value", &s));
        }
        RawPortValue::Int(n) if (1..=65535).contains(&n) => PortValue::Fixed(n as u16),
        RawPortValue::Int(n) => {
            return Err(ManifestError::new(
                ManifestErrorKind::InvalidValue,
                "runtime.ports.value",
                format!("port out of range: {n}"),
            ));
        }
    };
    Ok(Port {
        address: raw.address,
        value,
        inject: raw.inject,
    })
}

fn normalize_runtime_isolation(
    raw: RawRuntimeIsolation,
) -> Result<RuntimeIsolation, ManifestError> {
    Ok(RuntimeIsolation {
        profile: raw
            .profile
            .as_deref()
            .map(|p| match p {
                "none" => Ok(IsolationProfile::None),
                "observed" => Ok(IsolationProfile::Observed),
                "sandboxed" => Ok(IsolationProfile::Sandboxed),
                "strict" => Ok(IsolationProfile::Strict),
                other => Err(invalid("runtime.isolation.profile", other)),
            })
            .transpose()?,
        core_dumps: raw.core_dumps.as_deref().map(allow_deny).transpose()?,
        swap_secrets: raw.swap_secrets.as_deref().map(allow_deny).transpose()?,
        proc_visibility: raw
            .proc_visibility
            .as_deref()
            .map(|v| match v {
                "host" => Ok(ProcVisibility::Host),
                "restricted" => Ok(ProcVisibility::Restricted),
                other => Err(invalid("runtime.isolation.proc_visibility", other)),
            })
            .transpose()?,
        ptrace: raw.ptrace.as_deref().map(allow_deny).transpose()?,
    })
}

fn allow_deny(v: &str) -> Result<AllowDeny, ManifestError> {
    match v {
        "allow" => Ok(AllowDeny::Allow),
        "deny" => Ok(AllowDeny::Deny),
        other => Err(invalid("allow_deny", other)),
    }
}

fn normalize_check(path: &str, raw: RawCheck) -> Result<Check, ManifestError> {
    Ok(Check {
        check_type: match raw.check_type.as_str() {
            "process" => CheckType::Process,
            "tcp" => CheckType::Tcp,
            "http" => CheckType::Http,
            "command" => CheckType::Command,
            "fd" => CheckType::Fd,
            "external" => CheckType::External,
            other => return Err(invalid(&format!("{path}.type"), other)),
        },
        port: raw
            .port
            .as_deref()
            .map(|p| label(&format!("{path}.port"), p))
            .transpose()?,
        path: raw.path,
        command: raw.command,
        interval_ms: parse_duration_millis(&raw.interval)
            .map_err(|e| ManifestError::new(e.kind(), format!("{path}.interval"), e.message()))?,
        timeout_ms: parse_duration_millis(&raw.timeout)
            .map_err(|e| ManifestError::new(e.kind(), format!("{path}.timeout"), e.message()))?,
        initial_delay_ms: raw
            .initial_delay
            .as_deref()
            .map(|d| {
                parse_duration_millis(d).map_err(|e| {
                    ManifestError::new(e.kind(), format!("{path}.initial_delay"), e.message())
                })
            })
            .transpose()?,
        successes: raw.successes,
        failures: raw.failures,
    })
}

fn normalize_health(raw: RawHealth) -> Result<Health, ManifestError> {
    Ok(Health {
        readiness: raw
            .readiness
            .map(|c| normalize_check("health.readiness", c))
            .transpose()?,
        liveness: raw
            .liveness
            .map(|c| normalize_check("health.liveness", c))
            .transpose()?,
        progress: raw
            .progress
            .map(|c| normalize_check("health.progress", c))
            .transpose()?,
        completion: raw
            .completion
            .map(|c| normalize_check("health.completion", c))
            .transpose()?,
    })
}

fn normalize_resources(raw: RawResources) -> Result<Resources, ManifestError> {
    Ok(Resources {
        cpu_request: raw.cpu_request,
        cpu_limit: raw.cpu_limit,
        memory_request: raw
            .memory_request
            .as_deref()
            .map(|b| {
                parse_byte_size(b).map_err(|e| {
                    ManifestError::new(e.kind(), "resources.memory_request", e.message())
                })
            })
            .transpose()?,
        memory_limit: raw
            .memory_limit
            .as_deref()
            .map(|b| {
                parse_byte_size(b).map_err(|e| {
                    ManifestError::new(e.kind(), "resources.memory_limit", e.message())
                })
            })
            .transpose()?,
        ephemeral_request: raw
            .ephemeral_request
            .as_deref()
            .map(|b| {
                parse_byte_size(b).map_err(|e| {
                    ManifestError::new(e.kind(), "resources.ephemeral_request", e.message())
                })
            })
            .transpose()?,
        ephemeral_limit: raw
            .ephemeral_limit
            .as_deref()
            .map(|b| {
                parse_byte_size(b).map_err(|e| {
                    ManifestError::new(e.kind(), "resources.ephemeral_limit", e.message())
                })
            })
            .transpose()?,
        gpu_count: raw.gpu_count,
        gpu_vendor: raw.gpu_vendor,
        gpu_model: raw.gpu_model,
        gpu_memory_min: raw
            .gpu_memory_min
            .as_deref()
            .map(|b| {
                parse_byte_size(b).map_err(|e| {
                    ManifestError::new(e.kind(), "resources.gpu_memory_min", e.message())
                })
            })
            .transpose()?,
        capabilities: raw.capabilities,
    })
}

fn normalize_deploy(raw: RawDeploy) -> Result<Deploy, ManifestError> {
    let coverage = raw
        .coverage
        .as_deref()
        .map(|c| match c {
            "fixed" => Ok(Coverage::Fixed),
            "all_nodes" => Ok(Coverage::AllNodes),
            other => Err(invalid("deploy.coverage", other)),
        })
        .transpose()?;
    if coverage == Some(Coverage::AllNodes) && raw.units.is_some() {
        return Err(ManifestError::new(
            ManifestErrorKind::Semantic,
            "deploy",
            "coverage=all_nodes must not set units (gump.schema.json)",
        ));
    }
    Ok(Deploy {
        units: raw.units,
        coverage,
        priority: raw
            .priority
            .as_deref()
            .map(|p| match p {
                "low" => Ok(Priority::Low),
                "normal" => Ok(Priority::Normal),
                "high" => Ok(Priority::High),
                "critical" => Ok(Priority::Critical),
                other => Err(invalid("deploy.priority", other)),
            })
            .transpose()?,
        preemptible: raw.preemptible,
        rollout: raw.rollout.map(normalize_rollout).transpose()?,
        placement: raw.placement.map(normalize_placement).transpose()?,
    })
}

fn normalize_rollout(raw: RawRollout) -> Result<Rollout, ManifestError> {
    Ok(Rollout {
        strategy: raw
            .strategy
            .as_deref()
            .map(|s| match s {
                "replace" => Ok(RolloutStrategy::Replace),
                "rolling" => Ok(RolloutStrategy::Rolling),
                other => Err(invalid("deploy.rollout.strategy", other)),
            })
            .transpose()?,
        max_unavailable: raw.max_unavailable,
        max_surge: raw.max_surge,
    })
}

fn normalize_placement(raw: RawPlacement) -> Result<Placement, ManifestError> {
    Ok(Placement {
        spread: raw.spread,
        require: raw.require,
        prefer: raw.prefer,
    })
}

fn normalize_discovery(raw: RawDiscovery) -> Result<Discovery, ManifestError> {
    Ok(Discovery {
        hiccup: raw
            .hiccup
            .map(|h| {
                Ok(HiccupDiscovery {
                    required_for_eligibility: h.required_for_eligibility,
                    health_binding: h
                        .health_binding
                        .as_deref()
                        .map(|b| match b {
                            "readiness" => Ok(HealthBinding::Readiness),
                            "liveness" => Ok(HealthBinding::Liveness),
                            other => Err(invalid("discovery.hiccup.health_binding", other)),
                        })
                        .transpose()?,
                })
            })
            .transpose()?,
    })
}

fn normalize_publish(raw: RawPublish) -> Result<Publish, ManifestError> {
    Ok(Publish {
        provider: label("publish.provider", &raw.provider)?,
        required: raw.required,
        service: label("publish.service", &raw.service)?,
        port: label("publish.port", &raw.port)?,
        domain: raw.domain,
        protocol: raw
            .protocol
            .as_deref()
            .map(|p| match p {
                "tcp" => Ok(PublishProtocol::Tcp),
                "http" => Ok(PublishProtocol::Http),
                "https" => Ok(PublishProtocol::Https),
                other => Err(invalid("publish.protocol", other)),
            })
            .transpose()?,
    })
}

fn normalize_telemetry(raw: RawTelemetry) -> Result<Telemetry, ManifestError> {
    let protocol = raw.protocol.unwrap_or_else(|| "ratatouille/0.1".into());
    let format = raw.format.unwrap_or_else(|| "ndjson".into());
    if protocol != "ratatouille/0.1" {
        return Err(invalid("telemetry.protocol", &protocol));
    }
    if format != "ndjson" {
        return Err(invalid("telemetry.format", &format));
    }
    Ok(Telemetry {
        protocol,
        format,
        filter: raw.filter,
        relay: raw.relay.map(normalize_relay).transpose()?,
    })
}

fn normalize_relay(raw: RawRelay) -> Result<TelemetryRelay, ManifestError> {
    Ok(TelemetryRelay {
        capacity: raw
            .capacity
            .as_deref()
            .map(|b| {
                parse_byte_size(b).map_err(|e| {
                    ManifestError::new(e.kind(), "telemetry.relay.capacity", e.message())
                })
            })
            .transpose()?,
        max_record: raw
            .max_record
            .as_deref()
            .map(|b| {
                parse_byte_size(b).map_err(|e| {
                    ManifestError::new(e.kind(), "telemetry.relay.max_record", e.message())
                })
            })
            .transpose()?,
        overflow: raw
            .overflow
            .as_deref()
            .map(|o| match o {
                "drop_oldest" => Ok(Overflow::DropOldest),
                "drop_newest" => Ok(Overflow::DropNewest),
                other => Err(invalid("telemetry.relay.overflow", other)),
            })
            .transpose()?,
    })
}

fn normalize_local(raw: RawLocal) -> Result<Local, ManifestError> {
    let mut variables = BTreeMap::new();
    for (name, var) in raw.variables {
        validate_env_name("local.variables", &name)?;
        variables.insert(name, LocalVariable { source: var.source });
    }
    Ok(Local {
        watch: raw.watch,
        ports: raw.ports,
        variables,
    })
}

fn validate_env_name(path: &str, name: &str) -> Result<(), ManifestError> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 128 {
        return Err(ManifestError::new(
            ManifestErrorKind::InvalidValue,
            path,
            format!("invalid env name length for {name:?}"),
        ));
    }
    let first = bytes[0];
    if !(first.is_ascii_uppercase() || first == b'_') {
        return Err(ManifestError::new(
            ManifestErrorKind::InvalidValue,
            path,
            format!("env name must start with A-Z or _: {name:?}"),
        ));
    }
    if !bytes[1..]
        .iter()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'_')
    {
        return Err(ManifestError::new(
            ManifestErrorKind::InvalidValue,
            path,
            format!("invalid env name charset: {name:?}"),
        ));
    }
    Ok(())
}

fn invalid(path: &str, value: &str) -> ManifestError {
    ManifestError::new(
        ManifestErrorKind::InvalidValue,
        path,
        format!("unsupported value {value:?}"),
    )
}
