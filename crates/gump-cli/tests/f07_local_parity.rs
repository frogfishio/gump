//! F07 exit evidence: local parity for `gump run` and `gump test --sealed`.
//!
//! Authority: docs/v1/DELIVERY.md F07, DECISIONS D014, CONFORMANCE §6.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use gump_cli::{run_local, run_sealed_test, LocalRunOptions, SealedTestOptions};

fn tmp_workspace(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("gump-f07-{name}-{nanos}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_fixture(ws: &std::path::Path) {
    fs::create_dir_all(ws.join("bin")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::write(ws.join("bin/hello"), "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(ws.join("bin/hello"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(not(unix))]
    {
        fs::write(ws.join("bin/hello"), b"ok").unwrap();
    }
    fs::write(
        ws.join("gump.toml"),
        r#"
schema = "gump/1"

[app]
id = "hello-job"
namespace = "default"

[workload]
lifetime = "finite"
coordination = "independent"
success = "all_exit_zero"

[package]
root = "."
include = ["bin/hello"]
exclude = []

[runtime]
driver = "native"
command = ["./bin/hello"]

[telemetry]
protocol = "ratatouille/0.1"
format = "ndjson"
filter = "app/*"
"#,
    )
    .unwrap();
}

#[test]
fn run_local_parity_executes_same_command_vector() {
    let ws = tmp_workspace("run");
    write_fixture(&ws);
    let report = run_local(LocalRunOptions {
        workspace: ws.clone(),
        manifest_path: PathBuf::from("gump.toml"),
        state_root: Some(ws.join("state")),
    })
    .unwrap();
    assert_eq!(report.mode, "run");
    assert_eq!(report.namespace, "default");
    assert_eq!(report.app_id, "hello-job");
    assert_eq!(report.command_vector, vec!["bin/hello".to_string()]);
    assert_eq!(report.telemetry_filter.as_deref(), Some("app/*"));
    assert!(report.release_root.starts_with(ws.join("state").join("apps")));
    assert_eq!(report.exit_code, Some(0));
    let _ = fs::remove_dir_all(ws);
}

#[test]
fn sealed_test_builds_verifies_and_runs() {
    let ws = tmp_workspace("sealed");
    write_fixture(&ws);
    let report = run_sealed_test(SealedTestOptions {
        workspace: ws.clone(),
        manifest_path: PathBuf::from("gump.toml"),
        state_root: Some(ws.join("state")),
    })
    .unwrap();
    assert_eq!(report.mode, "test-sealed");
    assert_eq!(report.command_vector, vec!["bin/hello".to_string()]);
    assert_eq!(report.exit_code, Some(0));
    let _ = fs::remove_dir_all(ws);
}
