//! `gump` binary entry (F07).

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use gump_cli::{LocalRunOptions, LocalRunReport, SealedTestOptions, run_local, run_sealed_test};

fn main() -> ExitCode {
    match dispatch(env::args().skip(1).collect()) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("gump: {err}");
            ExitCode::from(2)
        }
    }
}

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
    Help,
}

fn dispatch(args: Vec<String>) -> Result<ExitCode, String> {
    let cmd = parse_args(&args)?;
    match cmd {
        Command::Help => {
            print_help();
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
    }
}

fn parse_args(args: &[String]) -> Result<Command, String> {
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        return Ok(Command::Help);
    }
    let mut iter = args.iter();
    let verb = iter.next().ok_or("missing command")?;
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
    match verb.as_str() {
        "run" => Ok(Command::Run {
            manifest,
            workspace,
        }),
        "test" => Ok(Command::Test {
            manifest,
            workspace,
            sealed,
        }),
        other => Err(format!("unknown command {other:?}; try gump run|test")),
    }
}

fn print_help() {
    eprintln!(
        "\
gump — local Capsule/runtime CLI (F07)

Usage:
  gump run [--manifest PATH] [--workspace DIR]
  gump test --sealed [--manifest PATH] [--workspace DIR]

`run` materializes an unsealed release and executes the driver contract.
`test --sealed` builds/verifies a local Capsule, then runs the same path.
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
