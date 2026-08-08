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
    },
    Help,
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
        "run" | "test" | "status" | "observe" | "deploy" | "lifecycle" | "recovery" | "cluster"
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
        } => {
            let client = LocalClient::new(socket);
            let deadline = deadline_ms.map(Duration::from_millis);
            let body = client.call(request, deadline).map_err(|e| e.to_string())?;
            let out = MachineOutputV1::wrap(body);
            println!("{}", out.to_canonical_json().map_err(|e| e.to_string())?);
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
        "observe" => {
            let mut socket = PathBuf::from("/tmp/gump.sock");
            let mut subject = "cluster".to_string();
            let mut deadline_ms = None;
            while let Some(a) = iter.next() {
                match a.as_str() {
                    "--socket" => {
                        socket = PathBuf::from(iter.next().ok_or("--socket needs a path")?);
                    }
                    "--subject" => {
                        subject = iter.next().ok_or("--subject needs a value")?.clone();
                    }
                    "--deadline-ms" => {
                        deadline_ms =
                            Some(parse_u64(iter.next().ok_or("--deadline-ms needs ms")?)?);
                    }
                    other => return Err(format!("unknown flag {other}")),
                }
            }
            Ok(Command::Api {
                socket,
                request: LocalRequest::Observe { subject },
                deadline_ms,
            })
        }
        "deploy" => parse_deploy(iter),
        "lifecycle" => {
            let action = iter
                .next()
                .ok_or("lifecycle needs action (cancel|interrupt|wait)")?
                .clone();
            let mut socket = PathBuf::from("/tmp/gump.sock");
            let mut subject = "attempt".to_string();
            let mut deadline_ms = None;
            while let Some(a) = iter.next() {
                match a.as_str() {
                    "--socket" => {
                        socket = PathBuf::from(iter.next().ok_or("--socket needs a path")?);
                    }
                    "--subject" => {
                        subject = iter.next().ok_or("--subject needs a value")?.clone();
                    }
                    "--deadline-ms" => {
                        deadline_ms =
                            Some(parse_u64(iter.next().ok_or("--deadline-ms needs ms")?)?);
                    }
                    other => return Err(format!("unknown flag {other}")),
                }
            }
            Ok(Command::Api {
                socket,
                request: LocalRequest::Lifecycle { action, subject },
                deadline_ms,
            })
        }
        "recovery" => {
            let action = iter.next().cloned().unwrap_or_else(|| "status".into());
            let mut socket = PathBuf::from("/tmp/gump.sock");
            let mut deadline_ms = None;
            while let Some(a) = iter.next() {
                match a.as_str() {
                    "--socket" => {
                        socket = PathBuf::from(iter.next().ok_or("--socket needs a path")?);
                    }
                    "--deadline-ms" => {
                        deadline_ms =
                            Some(parse_u64(iter.next().ok_or("--deadline-ms needs ms")?)?);
                    }
                    other => return Err(format!("unknown flag {other}")),
                }
            }
            Ok(Command::Api {
                socket,
                request: LocalRequest::Recovery {
                    action,
                    provider: None,
                    key_id: None,
                    recovery_secret_hex: None,
                },
                deadline_ms,
            })
        }
        "cluster" => {
            let action = iter.next().cloned().unwrap_or_else(|| "members".into());
            let mut socket = PathBuf::from("/tmp/gump.sock");
            let mut deadline_ms = None;
            while let Some(a) = iter.next() {
                match a.as_str() {
                    "--socket" => {
                        socket = PathBuf::from(iter.next().ok_or("--socket needs a path")?);
                    }
                    "--deadline-ms" => {
                        deadline_ms =
                            Some(parse_u64(iter.next().ok_or("--deadline-ms needs ms")?)?);
                    }
                    other => return Err(format!("unknown flag {other}")),
                }
            }
            Ok(Command::Api {
                socket,
                request: LocalRequest::ClusterAdmin { action },
                deadline_ms,
            })
        }
        other => Err(format!(
            "unknown command {other:?}; try gump run|test|status|observe|server"
        )),
    }
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
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--socket" => {
                socket = PathBuf::from(iter.next().ok_or("--socket needs a path")?);
            }
            "--deadline-ms" => {
                deadline_ms = Some(parse_u64(iter.next().ok_or("--deadline-ms needs ms")?)?);
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(Command::Api {
        socket,
        request,
        deadline_ms,
    })
}

fn parse_deploy<'a>(mut iter: impl Iterator<Item = &'a String>) -> Result<Command, String> {
    let mut socket = PathBuf::from("/tmp/gump.sock");
    let mut operation_id = String::new();
    let mut namespace = "default".to_string();
    let mut app = "app".to_string();
    let mut content_digest_hex = String::new();
    let mut capsule_hex = None;
    let mut deadline_ms = None;
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
            "--deadline-ms" => {
                deadline_ms = Some(parse_u64(iter.next().ok_or("--deadline-ms needs ms")?)?);
            }
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
        },
        deadline_ms,
    })
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
  gump status [--socket PATH] [--deadline-ms N]
  gump observe [--socket PATH] [--subject NAME] [--deadline-ms N]
  gump deploy --operation-id ID --digest HEX [--capsule-hex HEX] [--namespace NS] [--app APP] [--socket PATH]
  gump lifecycle cancel|interrupt|wait --subject NAME [--socket PATH]
  gump recovery [status|reseal] [--socket PATH]
  gump cluster [members|status] [--socket PATH]

API verbs are clients of the local Unix protocol (GUMP-N006); they do not duplicate
server semantics. Incompatible protocol versions fail with PROTOCOL_MISMATCH.
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
        LocalResponse::Error(_) => "error",
    }
}
