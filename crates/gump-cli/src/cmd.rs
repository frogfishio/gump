// SPDX-FileCopyrightText: 2026 Alexander R. Croft
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Local CLI verbs shared with the composed `gump` binary (GUMP-N004 / N006).

use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use crate::local_api::{LocalClient, LocalRequest, LocalResponse, MachineOutputV1};
use crate::{BootstrapInitializeOptions, initialize_from_handoff};
use crate::{LocalRunOptions, LocalRunReport, SealedTestOptions, run_local, run_sealed_test};
use crate::{build_sealed_capsule_for_cluster_os, local_parity_plan};
use gump_crypto::{ClusterX25519Public, SigningKeyBytes};
use gump_types::{CapsuleId, ClusterId};

#[derive(Debug)]
enum Command {
    Run {
        manifest: PathBuf,
        workspace: PathBuf,
    },
    Test {
        manifest: PathBuf,
        workspace: PathBuf,
        sealed: bool,
    },
    /// Cluster-backed local API clients (GUMP-N006) — no duplicated server logic.
    Api {
        socket: PathBuf,
        request: LocalRequest,
        deadline_ms: Option<u64>,
        /// `machine` (default JSON) or `human` (stable text; never prints secrets).
        format: OutputFormat,
    },
    CapsuleBuild {
        workspace: PathBuf,
        manifest: PathBuf,
        output: PathBuf,
        capsule_id: CapsuleId,
        cluster_id: ClusterId,
        cluster_public_key: [u8; 32],
        cluster_key_id: String,
        signing_key: SigningKeyBytes,
    },
    BootstrapInitialize(BootstrapInitializeOptions),
    Version,
    Copyright,
    Help,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputFormat {
    Machine,
    Human,
}

/// Dispatch CLI arguments. Returns `None` when the verb is owned by the process entry
/// (`server`).
pub fn try_dispatch_cli(args: &[String]) -> Option<Result<ExitCode, String>> {
    if args.is_empty() {
        return Some(Ok(print_help_ok()));
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        return Some(Ok(print_help_ok()));
    }
    if args.len() == 1 && matches!(args[0].as_str(), "-V" | "--version") {
        return Some(dispatch_cli(args));
    }
    if args.len() == 1 && matches!(args[0].as_str(), "--copyright" | "--coopyrigght") {
        return Some(dispatch_cli(args));
    }
    let verb = args[0].as_str();
    if !matches!(
        verb,
        "run"
            | "test"
            | "status"
            | "observe"
            | "deploy"
            | "lifecycle"
            | "recovery"
            | "cluster"
            | "telemetry"
            | "explain"
            | "inventory"
            | "inspect"
            | "reintroduce"
            | "capsule"
            | "bootstrap"
    ) {
        return None;
    }
    Some(dispatch_cli(args))
}

/// Run a CLI verb; errors if the verb is not a known client command.
pub fn dispatch_cli(args: &[String]) -> Result<ExitCode, String> {
    let cmd = parse_args(args)?;
    match cmd {
        Command::Help => Ok(print_help_ok()),
        Command::Version => {
            println!("{}", gump_types::product::version_string());
            Ok(ExitCode::SUCCESS)
        }
        Command::Copyright => {
            for line in gump_types::product::copyright_lines() {
                println!("{line}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Run {
            manifest,
            workspace,
        } => {
            let report = run_local(LocalRunOptions {
                workspace,
                manifest_path: manifest,
                state_root: None,
            })
            .map_err(|e| e.to_string())?;
            print_report(&report);
            Ok(exit_from_code(report.exit_code))
        }
        Command::Test {
            manifest,
            workspace,
            sealed,
        } => {
            if !sealed {
                return Err("gump test requires --sealed in v1 local parity (D014)".into());
            }
            let report = run_sealed_test(SealedTestOptions {
                workspace,
                manifest_path: manifest,
                state_root: None,
            })
            .map_err(|e| e.to_string())?;
            print_report(&report);
            Ok(exit_from_code(report.exit_code))
        }
        Command::Api {
            socket,
            request,
            deadline_ms,
            format,
        } => {
            let client = LocalClient::new(socket);
            let deadline = deadline_ms.map(Duration::from_millis);
            let body = client.call(request, deadline).map_err(|e| e.to_string())?;
            let exit = response_exit_code(&body);
            match format {
                OutputFormat::Machine => {
                    let out = MachineOutputV1::wrap(body);
                    println!("{}", out.to_canonical_json().map_err(|e| e.to_string())?);
                }
                OutputFormat::Human => print_human(&body),
            }
            Ok(exit)
        }
        Command::CapsuleBuild {
            workspace,
            manifest,
            output,
            capsule_id,
            cluster_id,
            cluster_public_key,
            cluster_key_id,
            signing_key,
        } => {
            let plan = local_parity_plan(&workspace, &manifest).map_err(|e| e.to_string())?;
            let built = build_sealed_capsule_for_cluster_os(
                &plan,
                capsule_id,
                cluster_id,
                &signing_key,
                &ClusterX25519Public(cluster_public_key),
                &cluster_key_id,
            )
            .map_err(|e| e.to_string())?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)
                .map_err(|e| format!("create Capsule {}: {e}", output.display()))?;
            file.write_all(&built.bytes)
                .map_err(|e| format!("write Capsule {}: {e}", output.display()))?;
            file.sync_all()
                .map_err(|e| format!("sync Capsule {}: {e}", output.display()))?;
            println!(
                "{{\"schema\":\"gump.capsule-build/1\",\"capsule_id\":\"{}\",\"cluster_id\":\"{}\",\"content_digest_hex\":\"{}\",\"size_bytes\":{},\"output\":{}}}",
                capsule_id,
                cluster_id,
                hex_encode(blake3::hash(&built.bytes).as_bytes()),
                built.bytes.len(),
                serde_json::to_string(&output.display().to_string()).map_err(|e| e.to_string())?
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::BootstrapInitialize(options) => {
            let result = initialize_from_handoff(options)?;
            println!(
                "{}",
                serde_json::to_string(&result).map_err(|e| e.to_string())?
            );
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn parse_args(args: &[String]) -> Result<Command, String> {
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        return Ok(Command::Help);
    }
    let mut iter = args.iter();
    let verb = iter.next().ok_or("missing command")?;
    match verb.as_str() {
        "-V" | "--version" if args.len() == 1 => Ok(Command::Version),
        "--copyright" | "--coopyrigght" if args.len() == 1 => Ok(Command::Copyright),
        "run" | "test" => parse_local_parity(verb, iter),
        "status" => parse_api_simple(iter, LocalRequest::Status),
        "explain" => {
            parse_subject_api(iter, "cluster", |subject| LocalRequest::Explain { subject })
        }
        "observe" => {
            parse_subject_api(iter, "cluster", |subject| LocalRequest::Observe { subject })
        }
        "deploy" => parse_deploy(iter),
        "lifecycle" => {
            let action = iter
                .next()
                .ok_or("lifecycle needs action (cancel|interrupt|wait)")?
                .clone();
            parse_subject_api(iter, "attempt", move |subject| LocalRequest::Lifecycle {
                action: action.clone(),
                subject,
            })
        }
        "recovery" => {
            let action = iter.next().cloned().unwrap_or_else(|| "status".into());
            parse_recovery(iter, action)
        }
        "cluster" => {
            let action = iter.next().cloned().unwrap_or_else(|| "members".into());
            parse_api_simple(iter, LocalRequest::ClusterAdmin { action })
        }
        "telemetry" => parse_telemetry(iter),
        "inventory" => parse_api_simple(iter, LocalRequest::Inventory),
        "inspect" => parse_inspect(iter),
        "reintroduce" => parse_reintroduce(iter),
        "capsule" => {
            let action = iter.next().ok_or("capsule needs action (build)")?;
            if action != "build" {
                return Err(format!("unknown capsule action {action:?}"));
            }
            parse_capsule_build(iter)
        }
        "bootstrap" => {
            let action = iter.next().ok_or("bootstrap needs action (initialize)")?;
            if action != "initialize" {
                return Err(format!("unknown bootstrap action {action:?}"));
            }
            parse_bootstrap_initialize(iter)
        }
        other => Err(format!(
            "unknown command {other:?}; try gump run|test|status|explain|observe|deploy|inventory|inspect|reintroduce|telemetry|server"
        )),
    }
}

fn parse_bootstrap_initialize<'a>(
    mut iter: impl Iterator<Item = &'a String>,
) -> Result<Command, String> {
    let mut handoff_fd = None;
    let mut activation_fd = None;
    let mut initialization_fd = None;
    let mut management_output_fd = None;
    let mut management_identity_ref = None;
    let mut deadline = Duration::from_secs(60);
    while let Some(argument) = iter.next() {
        let destination = match argument.as_str() {
            "--handoff-fd" => &mut handoff_fd,
            "--activation-fd" => &mut activation_fd,
            "--initialization-fd" => &mut initialization_fd,
            "--management-output-fd" => &mut management_output_fd,
            "--management-identity-ref" => {
                management_identity_ref = Some(
                    iter.next()
                        .ok_or("--management-identity-ref needs a value")?
                        .clone(),
                );
                continue;
            }
            "--deadline-ms" => {
                let milliseconds = parse_u64(iter.next().ok_or("--deadline-ms needs N")?)?;
                if milliseconds == 0 || milliseconds > 10 * 60 * 1000 {
                    return Err("--deadline-ms must be within 1..=600000".into());
                }
                deadline = Duration::from_millis(milliseconds);
                continue;
            }
            other => return Err(format!("unknown bootstrap initialize option {other:?}")),
        };
        let raw = parse_u64(
            iter.next()
                .ok_or_else(|| format!("{argument} needs a descriptor"))?,
        )?;
        if !(3..=i32::MAX as u64).contains(&raw) {
            return Err(format!("{argument} must be an inherited descriptor >= 3"));
        }
        *destination = Some(raw as i32);
    }
    Ok(Command::BootstrapInitialize(BootstrapInitializeOptions {
        handoff_fd: handoff_fd.ok_or("bootstrap initialize requires --handoff-fd")?,
        activation_fd: activation_fd.ok_or("bootstrap initialize requires --activation-fd")?,
        initialization_fd: initialization_fd
            .ok_or("bootstrap initialize requires --initialization-fd")?,
        management_output_fd: management_output_fd
            .ok_or("bootstrap initialize requires --management-output-fd")?,
        management_identity_ref: management_identity_ref
            .ok_or("bootstrap initialize requires --management-identity-ref")?,
        deadline,
    }))
}

fn parse_capsule_build<'a>(mut iter: impl Iterator<Item = &'a String>) -> Result<Command, String> {
    let workspace = env::current_dir().map_err(|e| e.to_string())?;
    let mut workspace_arg = workspace;
    let mut manifest = PathBuf::from("gump.toml");
    let mut output = None;
    let mut capsule_id = None;
    let mut cluster_id = None;
    let mut cluster_public_key = None;
    let mut cluster_key_id = None;
    let mut signing_key_fd = None;
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--workspace" => {
                workspace_arg = PathBuf::from(iter.next().ok_or("--workspace needs a path")?)
            }
            "--manifest" => manifest = PathBuf::from(iter.next().ok_or("--manifest needs a path")?),
            "--output" => output = Some(PathBuf::from(iter.next().ok_or("--output needs a path")?)),
            "--capsule-id" => {
                capsule_id = Some(
                    iter.next()
                        .ok_or("--capsule-id needs a UUIDv7")?
                        .parse::<CapsuleId>()
                        .map_err(|_| "--capsule-id must be a UUIDv7")?,
                )
            }
            "--cluster-id" => {
                cluster_id = Some(
                    iter.next()
                        .ok_or("--cluster-id needs a UUIDv7")?
                        .parse::<ClusterId>()
                        .map_err(|_| "--cluster-id must be a UUIDv7")?,
                )
            }
            "--cluster-public-key" => {
                cluster_public_key = Some(parse_lower_hex32(
                    iter.next().ok_or("--cluster-public-key needs 64 hex")?,
                    "--cluster-public-key",
                )?)
            }
            "--cluster-key-id" => {
                cluster_key_id = Some(iter.next().ok_or("--cluster-key-id needs a value")?.clone())
            }
            "--signing-key-fd" => {
                let fd = parse_u64(iter.next().ok_or("--signing-key-fd needs a descriptor")?)?;
                if !(3..=u16::MAX as u64).contains(&fd) {
                    return Err("--signing-key-fd must be an inherited descriptor >= 3".into());
                }
                signing_key_fd = Some(fd as u16);
            }
            other => return Err(format!("unknown capsule build option {other:?}")),
        }
    }
    let signing_hex =
        read_secret_fd(signing_key_fd.ok_or("capsule build requires --signing-key-fd")?)?;
    let signing_key =
        SigningKeyBytes::from_bytes(parse_lower_hex32(&signing_hex, "--signing-key-fd")?);
    Ok(Command::CapsuleBuild {
        workspace: workspace_arg,
        manifest,
        output: output.ok_or("capsule build requires --output")?,
        capsule_id: capsule_id.unwrap_or_else(CapsuleId::new),
        cluster_id: cluster_id.ok_or("capsule build requires --cluster-id")?,
        cluster_public_key: cluster_public_key
            .ok_or("capsule build requires --cluster-public-key")?,
        cluster_key_id: cluster_key_id.ok_or("capsule build requires --cluster-key-id")?,
        signing_key,
    })
}

fn parse_lower_hex32(value: &str, label: &str) -> Result<[u8; 32], String> {
    if value.len() != 64
        || !value
            .as_bytes()
            .iter()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    {
        return Err(format!("{label} must be 32 bytes of lowercase hex"));
    }
    let mut out = [0u8; 32];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| format!("{label} contains invalid hex"))?;
    }
    Ok(out)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn parse_recovery<'a>(
    mut iter: impl Iterator<Item = &'a String>,
    action: String,
) -> Result<Command, String> {
    let mut socket = PathBuf::from("/tmp/gump.sock");
    let mut deadline_ms = None;
    let mut format = OutputFormat::Machine;
    let mut provider = None;
    let mut key_id = None;
    let mut secret_fd = None;
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--socket" => socket = PathBuf::from(iter.next().ok_or("--socket needs a path")?),
            "--deadline-ms" => {
                deadline_ms = Some(parse_u64(iter.next().ok_or("--deadline-ms needs N")?)?)
            }
            "--format" => format = parse_format(iter.next().ok_or("--format needs a value")?)?,
            "--provider" => provider = Some(iter.next().ok_or("--provider needs a value")?.clone()),
            "--key-id" => key_id = Some(iter.next().ok_or("--key-id needs a value")?.clone()),
            "--secret-fd" => {
                let fd = parse_u64(iter.next().ok_or("--secret-fd needs a descriptor")?)?;
                if !(3..=u16::MAX as u64).contains(&fd) {
                    return Err("--secret-fd must be an inherited descriptor >= 3".into());
                }
                secret_fd = Some(fd as u16);
            }
            other => return Err(format!("unknown recovery option {other:?}")),
        }
    }
    let recovery_secret_hex = secret_fd.map(read_secret_fd).transpose()?;
    if action == "unseal" && recovery_secret_hex.is_none() && provider.as_deref() != Some("hsm") {
        return Err("recovery unseal requires --secret-fd for the software provider".into());
    }
    Ok(Command::Api {
        socket,
        request: LocalRequest::Recovery {
            action,
            provider,
            key_id,
            recovery_secret_hex,
        },
        deadline_ms,
        format,
    })
}

fn read_secret_fd(fd: u16) -> Result<String, String> {
    // Do not re-open through /dev/fd: socket descriptors cannot be reopened,
    // and hardened Linux processes may deliberately restrict procfs.
    let mut bytes = gump_types::inherited_fd::read_bounded(i32::from(fd), 66)
        .map_err(|e| format!("read --secret-fd: {e}"))?;
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes.pop();
    }
    let out = if bytes.len() == 32 {
        let mut encoded = String::with_capacity(64);
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for &b in &bytes {
            encoded.push(HEX[(b >> 4) as usize] as char);
            encoded.push(HEX[(b & 0x0f) as usize] as char);
        }
        encoded
    } else if bytes.len() == 64
        && bytes
            .iter()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    {
        String::from_utf8(bytes.clone()).map_err(|_| "secret fd is not valid hex".to_string())?
    } else {
        bytes.fill(0);
        return Err("secret fd must contain exactly 32 raw bytes or 64 lowercase hex bytes".into());
    };
    bytes.fill(0);
    Ok(out)
}

fn parse_telemetry<'a>(mut iter: impl Iterator<Item = &'a String>) -> Result<Command, String> {
    let mut socket = PathBuf::from("/tmp/gump.sock");
    let mut filter = None;
    let mut max_events = None;
    let mut deadline_ms = None;
    let mut format = OutputFormat::Machine;
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--socket" => {
                socket = PathBuf::from(iter.next().ok_or("--socket needs a path")?);
            }
            "--filter" => {
                filter = Some(iter.next().ok_or("--filter needs a topic")?.clone());
            }
            "--max-events" => {
                let n = parse_u64(iter.next().ok_or("--max-events needs a count")?)?;
                max_events = Some(u32::try_from(n).map_err(|_| "max-events too large")?);
            }
            "--deadline-ms" => {
                deadline_ms = Some(parse_u64(iter.next().ok_or("--deadline-ms needs ms")?)?);
            }
            "--format" => {
                format = parse_format(iter.next().ok_or("--format needs machine|human")?)?;
            }
            "--human" => format = OutputFormat::Human,
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(Command::Api {
        socket,
        request: LocalRequest::Telemetry { filter, max_events },
        deadline_ms,
        format,
    })
}

fn parse_local_parity<'a>(
    verb: &str,
    mut iter: impl Iterator<Item = &'a String>,
) -> Result<Command, String> {
    let mut manifest = PathBuf::from("gump.toml");
    let mut workspace = env::current_dir().map_err(|e| e.to_string())?;
    let mut sealed = false;
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--manifest" => {
                let v = iter.next().ok_or("--manifest needs a path")?;
                manifest = PathBuf::from(v);
            }
            "--workspace" => {
                let v = iter.next().ok_or("--workspace needs a path")?;
                workspace = PathBuf::from(v);
            }
            "--sealed" => sealed = true,
            other if other.starts_with('-') => {
                return Err(format!("unknown flag {other}"));
            }
            other => {
                manifest = PathBuf::from(other);
            }
        }
    }
    match verb {
        "run" => Ok(Command::Run {
            manifest,
            workspace,
        }),
        "test" => Ok(Command::Test {
            manifest,
            workspace,
            sealed,
        }),
        _ => unreachable!(),
    }
}

fn parse_api_simple<'a>(
    mut iter: impl Iterator<Item = &'a String>,
    request: LocalRequest,
) -> Result<Command, String> {
    let mut socket = PathBuf::from("/tmp/gump.sock");
    let mut deadline_ms = None;
    let mut format = OutputFormat::Machine;
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--socket" => {
                socket = PathBuf::from(iter.next().ok_or("--socket needs a path")?);
            }
            "--deadline-ms" => {
                deadline_ms = Some(parse_u64(iter.next().ok_or("--deadline-ms needs ms")?)?);
            }
            "--format" => {
                format = parse_format(iter.next().ok_or("--format needs machine|human")?)?;
            }
            "--human" => format = OutputFormat::Human,
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(Command::Api {
        socket,
        request,
        deadline_ms,
        format,
    })
}

fn parse_subject_api<'a>(
    mut iter: impl Iterator<Item = &'a String>,
    default_subject: &str,
    build: impl FnOnce(String) -> LocalRequest,
) -> Result<Command, String> {
    let mut socket = PathBuf::from("/tmp/gump.sock");
    let mut subject = default_subject.to_string();
    let mut deadline_ms = None;
    let mut format = OutputFormat::Machine;
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--socket" => {
                socket = PathBuf::from(iter.next().ok_or("--socket needs a path")?);
            }
            "--subject" => {
                subject = iter.next().ok_or("--subject needs a value")?.clone();
            }
            "--deadline-ms" => {
                deadline_ms = Some(parse_u64(iter.next().ok_or("--deadline-ms needs ms")?)?);
            }
            "--format" => {
                format = parse_format(iter.next().ok_or("--format needs machine|human")?)?;
            }
            "--human" => format = OutputFormat::Human,
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(Command::Api {
        socket,
        request: build(subject),
        deadline_ms,
        format,
    })
}

fn parse_deploy<'a>(mut iter: impl Iterator<Item = &'a String>) -> Result<Command, String> {
    let mut socket = PathBuf::from("/tmp/gump.sock");
    let mut operation_id = String::new();
    let mut namespace = "default".to_string();
    let mut app = "app".to_string();
    let mut content_digest_hex = String::new();
    let mut capsule_hex = None;
    let mut capsule_path = None;
    let mut wait = None;
    let mut deadline_ms = None;
    let mut format = OutputFormat::Machine;
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--socket" => {
                socket = PathBuf::from(iter.next().ok_or("--socket needs a path")?);
            }
            "--operation-id" => {
                operation_id = iter.next().ok_or("--operation-id needs a value")?.clone();
            }
            "--namespace" => {
                namespace = iter.next().ok_or("--namespace needs a value")?.clone();
            }
            "--app" => {
                app = iter.next().ok_or("--app needs a value")?.clone();
            }
            "--digest" => {
                content_digest_hex = iter.next().ok_or("--digest needs hex")?.clone();
            }
            "--capsule-hex" => {
                capsule_hex = Some(iter.next().ok_or("--capsule-hex needs hex")?.clone());
            }
            "--capsule" => {
                capsule_path = Some(iter.next().ok_or("--capsule needs a path")?.clone());
            }
            "--wait" => {
                wait = Some(iter.next().ok_or("--wait needs a condition")?.clone());
            }
            "--deadline-ms" => {
                deadline_ms = Some(parse_u64(iter.next().ok_or("--deadline-ms needs ms")?)?);
            }
            "--format" => {
                format = parse_format(iter.next().ok_or("--format needs machine|human")?)?;
            }
            "--human" => format = OutputFormat::Human,
            other => return Err(format!("unknown flag {other}")),
        }
    }
    if operation_id.is_empty() || content_digest_hex.is_empty() {
        return Err("deploy requires --operation-id and --digest".into());
    }
    if capsule_hex.is_some() == capsule_path.is_some() {
        return Err("deploy requires exactly one of --capsule PATH or --capsule-hex HEX".into());
    }
    Ok(Command::Api {
        socket,
        request: LocalRequest::Deploy {
            operation_id,
            namespace,
            app,
            content_digest_hex,
            capsule_hex,
            capsule_path,
            wait,
        },
        deadline_ms,
        format,
    })
}

fn parse_inspect<'a>(mut iter: impl Iterator<Item = &'a String>) -> Result<Command, String> {
    let mut socket = PathBuf::from("/tmp/gump.sock");
    let mut capsule_id = None;
    let mut deadline_ms = None;
    let mut format = OutputFormat::Machine;
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--socket" => {
                socket = PathBuf::from(iter.next().ok_or("--socket needs a path")?);
            }
            "--capsule" | "--id" => {
                capsule_id = Some(iter.next().ok_or("--capsule needs a uuid")?.clone());
            }
            "--deadline-ms" => {
                deadline_ms = Some(parse_u64(iter.next().ok_or("--deadline-ms needs ms")?)?);
            }
            "--format" => {
                format = parse_format(iter.next().ok_or("--format needs machine|human")?)?;
            }
            "--human" => format = OutputFormat::Human,
            other if other.starts_with('-') => return Err(format!("unknown flag {other}")),
            other => capsule_id = Some(other.to_string()),
        }
    }
    let capsule_id = capsule_id.ok_or("inspect requires a capsule uuid")?;
    Ok(Command::Api {
        socket,
        request: LocalRequest::Inspect { capsule_id },
        deadline_ms,
        format,
    })
}

fn parse_reintroduce<'a>(mut iter: impl Iterator<Item = &'a String>) -> Result<Command, String> {
    let mut socket = PathBuf::from("/tmp/gump.sock");
    let mut capsule_id = None;
    let mut plan = false;
    let mut finite_mode = None;
    let mut resume_from = None;
    let mut operation_id = None;
    let mut namespace = None;
    let mut app = None;
    let mut deadline_ms = None;
    let mut format = OutputFormat::Machine;
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--socket" => {
                socket = PathBuf::from(iter.next().ok_or("--socket needs a path")?);
            }
            "--capsule" | "--id" => {
                capsule_id = Some(iter.next().ok_or("--capsule needs a uuid")?.clone());
            }
            "--plan" => plan = true,
            "--new-execution" => finite_mode = Some("new_execution".into()),
            "--resume-from" => {
                resume_from = Some(
                    iter.next()
                        .ok_or("--resume-from needs a reference")?
                        .clone(),
                );
                finite_mode = Some("resume".into());
            }
            "--operation-id" => {
                operation_id = Some(iter.next().ok_or("--operation-id needs a value")?.clone());
            }
            "--namespace" => {
                namespace = Some(iter.next().ok_or("--namespace needs a value")?.clone());
            }
            "--app" => {
                app = Some(iter.next().ok_or("--app needs a value")?.clone());
            }
            "--deadline-ms" => {
                deadline_ms = Some(parse_u64(iter.next().ok_or("--deadline-ms needs ms")?)?);
            }
            "--format" => {
                format = parse_format(iter.next().ok_or("--format needs machine|human")?)?;
            }
            "--human" => format = OutputFormat::Human,
            other if other.starts_with('-') => return Err(format!("unknown flag {other}")),
            other => capsule_id = Some(other.to_string()),
        }
    }
    let capsule_id = capsule_id.ok_or("reintroduce requires a capsule uuid")?;
    Ok(Command::Api {
        socket,
        request: LocalRequest::Reintroduce {
            capsule_id,
            plan,
            finite_mode,
            resume_from,
            operation_id,
            namespace,
            app,
        },
        deadline_ms,
        format,
    })
}

fn parse_format(s: &str) -> Result<OutputFormat, String> {
    match s {
        "machine" | "json" => Ok(OutputFormat::Machine),
        "human" | "text" => Ok(OutputFormat::Human),
        other => Err(format!("unknown --format {other:?}; use machine|human")),
    }
}

fn print_human(body: &LocalResponse) {
    match body {
        LocalResponse::Status(s) => {
            println!("cluster {}", s.cluster_id);
            println!("controller_epoch {}", s.controller_epoch);
            println!("memory_voters {}", s.memory_voters);
            println!("durability {}", s.durability_note);
        }
        LocalResponse::Explain {
            subject,
            reason_code,
            message,
            observation_source,
            compaction_disclosed,
            durability_note,
        } => {
            println!("subject {subject}");
            println!("reason {reason_code}");
            println!("source {observation_source}");
            println!("compaction_disclosed {compaction_disclosed}");
            println!("durability {durability_note}");
            println!("{message}");
        }
        LocalResponse::Deploy {
            operation_id,
            phase,
            reason_code,
            safe_message,
            desired_generation,
            content_digest_hex,
            durability_note,
            wait,
            stages,
            interrupted_implies_rollback,
        } => {
            println!("deploy {operation_id}");
            println!("phase {phase}");
            println!("reason {reason_code}");
            println!("digest {content_digest_hex}");
            if let Some(g) = desired_generation {
                println!("desired_generation {g}");
            }
            println!("durability {durability_note}");
            println!(
                "wait {} (default {})",
                wait.condition, wait.default_for_contract
            );
            println!("interrupted_implies_rollback {interrupted_implies_rollback}");
            for st in stages {
                println!("stage {}={} — {}", st.name, st.status, st.detail);
            }
            println!("{safe_message}");
        }
        LocalResponse::Lifecycle {
            action,
            subject,
            state,
            interrupted_implies_rollback,
            note,
        } => {
            println!("lifecycle {action} subject={subject} state={state}");
            println!("interrupted_implies_rollback {interrupted_implies_rollback}");
            if let Some(n) = note {
                println!("{n}");
            }
        }
        LocalResponse::Telemetry {
            profile,
            memory_only,
            pushed,
            dropped_oldest,
            caught_up,
            identity_note,
            events,
            ..
        } => {
            println!("telemetry profile={profile} memory_only={memory_only}");
            println!("pushed={pushed} dropped_oldest={dropped_oldest} caught_up={caught_up}");
            println!("{identity_note}");
            for ev in events {
                match ev {
                    crate::local_api::TelemetryEventBody::Record {
                        topic,
                        stream_sequence,
                        text,
                        ..
                    } => {
                        // Prefer text when present; never invent secrets from hex.
                        if let Some(t) = text {
                            println!("record {topic}#{stream_sequence} {t}");
                        } else {
                            println!("record {topic}#{stream_sequence} <binary>");
                        }
                    }
                    crate::local_api::TelemetryEventBody::Gap {
                        topic,
                        from_sequence,
                        to_sequence,
                        reason,
                    } => println!("gap {topic} {from_sequence}..{to_sequence} ({reason})"),
                }
            }
        }
        LocalResponse::Inventory {
            desired_count,
            note,
            capsules,
        } => {
            println!("inventory desired_count={desired_count}");
            println!("{note}");
            for c in capsules {
                println!(
                    "capsule {} digest={} size={} live_referenced={} inert={}",
                    c.capsule_id, c.content_digest_hex, c.size_bytes, c.live_referenced, c.inert
                );
            }
        }
        LocalResponse::Inspect {
            capsule_id,
            content_digest_hex,
            size_bytes,
            object_key,
            live_referenced,
            public_note,
        } => {
            println!("inspect {capsule_id}");
            println!("digest {content_digest_hex}");
            println!("size_bytes {size_bytes}");
            println!("object_key {object_key}");
            println!("live_referenced {live_referenced}");
            println!("{public_note}");
        }
        LocalResponse::Reintroduce {
            capsule_id,
            plan,
            phase,
            reason_code,
            safe_message,
            content_digest_hex,
            finite_mode,
            desired_generation,
            durability_note,
            restores_prior_desired,
        } => {
            println!("reintroduce {capsule_id} plan={plan}");
            println!("phase {phase}");
            println!("reason {reason_code}");
            println!("digest {content_digest_hex}");
            if let Some(m) = finite_mode {
                println!("finite_mode {m}");
            }
            if let Some(g) = desired_generation {
                println!("desired_generation {g}");
            }
            println!("durability {durability_note}");
            println!("restores_prior_desired {restores_prior_desired}");
            println!("{safe_message}");
        }
        LocalResponse::Error(e) => {
            println!("error {} ({})", e.code, e.reason);
            println!("{}", e.safe_message);
        }
        other => {
            // Fallback: machine JSON without claiming human formatting for rare kinds.
            let out = MachineOutputV1::wrap(other.clone());
            if let Ok(json) = out.to_canonical_json() {
                println!("{json}");
            }
        }
    }
}

fn parse_u64(s: &str) -> Result<u64, String> {
    s.parse::<u64>()
        .map_err(|_| format!("invalid integer {s:?}"))
}

fn print_help_ok() -> ExitCode {
    print_help();
    ExitCode::SUCCESS
}

pub fn print_help() {
    eprintln!(
        "\
gump — Capsule placer/supervisor (CLI + server roles)

Usage:
  gump --help
  gump --version
  gump --copyright
  gump run [--manifest PATH] [--workspace DIR]
  gump test --sealed [--manifest PATH] [--workspace DIR]
  gump capsule build --output PATH --cluster-id UUID --cluster-public-key HEX
              --cluster-key-id ID --signing-key-fd N [--manifest PATH] [--workspace DIR]
  gump cluster-material --nodes N [--cluster-id UUID]
  gump bootstrap initialize --handoff-fd N --activation-fd N --initialization-fd N
              --management-output-fd N --management-identity-ref REF [--deadline-ms N]
  gump server (--init | --join IP:PORT) --params-fd N [--state-root PATH] [--socket PATH] [--role ROLE[,ROLE...]]
  gump server --bootstrap --bootstrap-bind IP:PORT --advertise-bootstrap HTTPS_ORIGIN
              --management-bind IP:PORT --advertise-management HTTPS_ORIGIN
              [--runtime-directory PATH] [--state-root PATH] [--socket PATH]
  gump status [--socket PATH] [--deadline-ms N] [--format machine|human]
  gump explain [--subject NAME] [--socket PATH] [--format machine|human]
  gump observe [--socket PATH] [--subject NAME] [--deadline-ms N] [--format machine|human]
  gump deploy --operation-id ID --digest HEX --capsule PATH [--wait CONDITION]
              [--namespace NS] [--app APP] [--socket PATH] [--format machine|human]
  gump lifecycle cancel|interrupt|wait --subject NAME [--socket PATH] [--format machine|human]
  gump inventory [--socket PATH] [--format machine|human]
  gump inspect <capsule-uuid> [--socket PATH] [--format machine|human]
  gump reintroduce <capsule-uuid> (--new-execution | --resume-from REF) [--plan]
              [--operation-id ID] [--namespace NS] [--app APP] [--socket PATH] [--format machine|human]
  gump recovery [status|reseal] [--socket PATH]
  gump recovery unseal --secret-fd N [--provider software] [--key-id ID] [--socket PATH]
  gump cluster [members|status] [--socket PATH]
  gump telemetry [--filter TOPIC|prefix*] [--max-events N] [--socket PATH] [--format machine|human]

API verbs are clients of the local Unix protocol (GUMP-N006); they do not duplicate
server semantics. Incompatible protocol versions fail with PROTOCOL_MISMATCH.
Deploy receipts distinguish persistence vs intent vs later observed stages (GUMP-N015).
Default --wait is intent_accepted. Interrupt/cancel never imply Capsule rollback.
Telemetry is memory-only recent-window state; it is not a durable log.
Human format never prints recovery secrets or Capsule ciphertext.
Inventory/inspect never activate Capsules. Reintroduce creates fresh intent only
(GUMP-N016); finite work requires --new-execution or --resume-from.

Version format is VERSION+build-BUILD. Licensing is AGPL-3.0-or-later;
commercial licensing is available at https://frogfish.io.
"
    );
}

fn print_report(report: &LocalRunReport) {
    println!("mode={}", report.mode);
    println!("app={}/{}", report.namespace, report.app_id);
    println!("capsule_id={}", report.capsule_id);
    println!("release_root={}", report.release_root.display());
    println!("command={}", report.command_vector.join(" "));
    println!("workdir={}", report.workdir.display());
    if let Some(topics) = &report.telemetry_filter {
        println!("telemetry_filter={topics}");
    }
    println!(
        "exit={}",
        report
            .exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "null".into())
    );
}

fn exit_from_code(code: Option<i32>) -> ExitCode {
    match code {
        Some(0) => ExitCode::SUCCESS,
        Some(c) if (1..=255).contains(&c) => ExitCode::from(c as u8),
        _ => ExitCode::from(1),
    }
}

fn response_exit_code(response: &LocalResponse) -> ExitCode {
    if matches!(response, LocalResponse::Error(_)) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

// Silence unused warning if LocalResponse pattern matching expands later.
#[allow(dead_code)]
fn _response_kind(r: &LocalResponse) -> &'static str {
    match r {
        LocalResponse::Hello { .. } => "hello",
        LocalResponse::Status(_) => "status",
        LocalResponse::Explain { .. } => "explain",
        LocalResponse::Observe { .. } => "observe",
        LocalResponse::Deploy { .. } => "deploy",
        LocalResponse::Lifecycle { .. } => "lifecycle",
        LocalResponse::Recovery { .. } => "recovery",
        LocalResponse::ClusterAdmin { .. } => "cluster_admin",
        LocalResponse::Telemetry { .. } => "telemetry",
        LocalResponse::Inventory { .. } => "inventory",
        LocalResponse::Inspect { .. } => "inspect",
        LocalResponse::Reintroduce { .. } => "reintroduce",
        LocalResponse::Error(_) => "error",
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::os::fd::AsRawFd as _;
    use std::os::unix::net::UnixStream;
    use std::process::ExitCode;

    use super::{Command, parse_args, read_secret_fd, response_exit_code};
    use crate::local_api::{ErrorBody, LocalResponse};

    #[test]
    fn inherited_secret_fd_is_consumed_directly() {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        writer.write_all(&[0x5a; 32]).unwrap();
        drop(writer);
        let encoded = read_secret_fd(u16::try_from(reader.as_raw_fd()).unwrap()).unwrap();
        assert_eq!(encoded, "5a".repeat(32));
    }

    #[test]
    fn server_error_response_sets_failure_exit_status() {
        let response = LocalResponse::Error(ErrorBody {
            code: "CONFLICT".into(),
            reason: "deploy.conflict".into(),
            safe_message: "deployment conflicted".into(),
        });
        assert_eq!(response_exit_code(&response), ExitCode::from(1));
        assert_eq!(
            response_exit_code(&LocalResponse::Status(crate::local_api::sample_status())),
            ExitCode::SUCCESS
        );
    }

    #[test]
    fn global_product_information_flags_parse_exactly() {
        assert!(matches!(
            parse_args(&["--version".into()]).unwrap(),
            Command::Version
        ));
        assert!(matches!(
            parse_args(&["--copyright".into()]).unwrap(),
            Command::Copyright
        ));
        assert!(matches!(
            parse_args(&["--coopyrigght".into()]).unwrap(),
            Command::Copyright
        ));
        assert!(parse_args(&["--version".into(), "extra".into()]).is_err());
    }
}
