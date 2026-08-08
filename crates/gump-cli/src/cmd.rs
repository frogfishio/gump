//! Local CLI verbs shared with the composed `gump` binary (GUMP-N004 / N006).

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use crate::local_api::{LocalClient, LocalRequest, LocalResponse, MachineOutputV1};
use crate::{LocalRunOptions, LocalRunReport, SealedTestOptions, run_local, run_sealed_test};

#[derive(Clone, Debug, Eq, PartialEq)]
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
            match format {
                OutputFormat::Machine => {
                    let out = MachineOutputV1::wrap(body);
                    println!("{}", out.to_canonical_json().map_err(|e| e.to_string())?);
                }
                OutputFormat::Human => print_human(&body),
            }
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
            parse_api_simple(
                iter,
                LocalRequest::Recovery {
                    action,
                    provider: None,
                    key_id: None,
                    recovery_secret_hex: None,
                },
            )
        }
        "cluster" => {
            let action = iter.next().cloned().unwrap_or_else(|| "members".into());
            parse_api_simple(iter, LocalRequest::ClusterAdmin { action })
        }
        "telemetry" => parse_telemetry(iter),
        other => Err(format!(
            "unknown command {other:?}; try gump run|test|status|explain|observe|deploy|telemetry|server"
        )),
    }
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
    Ok(Command::Api {
        socket,
        request: LocalRequest::Deploy {
            operation_id,
            namespace,
            app,
            content_digest_hex,
            capsule_hex,
            wait,
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
  gump run [--manifest PATH] [--workspace DIR]
  gump test --sealed [--manifest PATH] [--workspace DIR]
  gump server --init [--socket PATH] [--role ROLE[,ROLE...]]
  gump status [--socket PATH] [--deadline-ms N] [--format machine|human]
  gump explain [--subject NAME] [--socket PATH] [--format machine|human]
  gump observe [--socket PATH] [--subject NAME] [--deadline-ms N] [--format machine|human]
  gump deploy --operation-id ID --digest HEX [--capsule-hex HEX] [--wait CONDITION]
              [--namespace NS] [--app APP] [--socket PATH] [--format machine|human]
  gump lifecycle cancel|interrupt|wait --subject NAME [--socket PATH] [--format machine|human]
  gump recovery [status|reseal] [--socket PATH]
  gump cluster [members|status] [--socket PATH]
  gump telemetry [--filter TOPIC|prefix*] [--max-events N] [--socket PATH] [--format machine|human]

API verbs are clients of the local Unix protocol (GUMP-N006); they do not duplicate
server semantics. Incompatible protocol versions fail with PROTOCOL_MISMATCH.
Deploy receipts distinguish persistence vs intent vs later observed stages (GUMP-N015).
Default --wait is intent_accepted. Interrupt/cancel never imply Capsule rollback.
Telemetry is memory-only recent-window state; it is not a durable log.
Human format never prints recovery secrets or Capsule ciphertext.
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
        LocalResponse::Error(_) => "error",
    }
}
