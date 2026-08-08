//! CLI entry for CI: validate `spec/v1/traceability.tsv`.
//!
//! Usage:
//!   check-traceability                 # structural + ticket ownership (default)
//!   check-traceability --strict        # fail on missing/blocked (release)
//!   check-traceability --prove-missing # exit 1 iff ledger has missing (W04 demo)

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use gump_gates::traceability::{Mode, check_file, default_owner_crates, parse_tsv};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let strict = args.iter().any(|a| a == "--strict");
    let prove_missing = args.iter().any(|a| a == "--prove-missing");
    let path = workspace_root().join("spec/v1/traceability.tsv");
    let crates = default_owner_crates();

    if prove_missing {
        // W04 exit evidence helper: succeed (exit 0) only when strict mode fails
        // because of missing/blocked rows — i.e. the gate demonstrably trips.
        let report = match check_file(&path, &crates, Mode::Strict) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("check-traceability: {e}");
                return ExitCode::from(2);
            }
        };
        if report.ok() {
            eprintln!(
                "check-traceability --prove-missing: expected strict failure, but ledger is fully covered"
            );
            return ExitCode::from(1);
        }
        if !report.errors.iter().any(|e| e.contains("strict:")) {
            eprintln!("check-traceability --prove-missing: failed for other reasons:\n{report}");
            return ExitCode::from(1);
        }
        println!("check-traceability: proved missing/blocked requirements fail strict gate");
        println!("{report}");
        return ExitCode::SUCCESS;
    }

    let mode = if strict {
        Mode::Strict
    } else {
        Mode::Structural
    };
    let report = match check_file(&path, &crates, mode) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("check-traceability: {e}");
            return ExitCode::from(2);
        }
    };

    // Always parse to show row count on success.
    let _ = parse_tsv(&std::fs::read_to_string(&path).unwrap_or_default());

    if report.ok() {
        println!("{report} ({mode:?})");
        ExitCode::SUCCESS
    } else {
        eprintln!("{report}");
        ExitCode::from(1)
    }
}
