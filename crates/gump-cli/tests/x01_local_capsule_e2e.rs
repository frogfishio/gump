//! X01 exit evidence: Local manifest → deterministic Capsule → verified local run.
//!
//! Authority: docs/v1/DELIVERY.md §8 slice 1, CONFORMANCE §6 Local parity, D014.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use gump_cli::{
    LocalRunOptions, build_sealed_capsule, local_parity_plan, run_local, run_verified_sealed,
    verify_sealed_capsule,
};
use gump_types::{CapsuleId, ClusterId};
use rand_core::{TryCryptoRng, TryRng};

/// Deterministic byte-stream RNG for Capsule reproducibility (X01).
struct ReplayRng {
    bytes: &'static [u8],
}

impl ReplayRng {
    fn new(bytes: &'static [u8]) -> Self {
        Self { bytes }
    }
}

impl TryRng for ReplayRng {
    type Error = core::convert::Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        rand_core::utils::next_word_via_fill(self)
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        rand_core::utils::next_word_via_fill(self)
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Self::Error> {
        assert!(
            dest.len() <= self.bytes.len(),
            "ReplayRng exhausted (need {}, have {})",
            dest.len(),
            self.bytes.len()
        );
        let (taken, rest) = self.bytes.split_at(dest.len());
        dest.copy_from_slice(taken);
        self.bytes = rest;
        Ok(())
    }
}

impl TryCryptoRng for ReplayRng {}

/// Fixed UUIDv7 wire bytes (version=7, RFC variant).
fn fixed_v7(tag: u8) -> [u8; 16] {
    let mut b = [
        0x01, 0x8f, 0x4a, 0x00, 0x00, 0x00, 0x70, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ];
    b[15] = tag;
    b
}

/// Plenty of entropy for signing + DEK/nonce + X25519 + HPKE ephemeral.
const X01_RNG_SEED: &[u8] = &[0xA5; 512];

fn tmp_workspace(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("gump-x01-{name}-{nanos}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_fixture(ws: &std::path::Path) {
    fs::create_dir_all(ws.join("bin")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::write(ws.join("bin/hello"), "#!/bin/sh\necho x01-ok\nexit 0\n").unwrap();
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
id = "x01-hello"
namespace = "ci"

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
fn archive_from_manifest_is_deterministic() {
    let ws = tmp_workspace("archive");
    write_fixture(&ws);
    let a = local_parity_plan(&ws, &PathBuf::from("gump.toml")).unwrap();
    let b = local_parity_plan(&ws, &PathBuf::from("gump.toml")).unwrap();
    let a_bytes = a.read_archive_bytes().unwrap();
    let b_bytes = b.read_archive_bytes().unwrap();
    assert_eq!(a_bytes, b_bytes);
    assert!(!a_bytes.is_empty());
    assert!(a.archive_spill_path().is_file());
    assert!(b.archive_spill_path().is_file());
    let _ = fs::remove_dir_all(ws);
}

#[test]
fn local_plan_packs_to_spill_and_run_streams_read() {
    // STL-26: compressed archive lives on a private spill; run_local opens it as Read.
    let ws = tmp_workspace("spill-run");
    write_fixture(&ws);
    let plan = local_parity_plan(&ws, &PathBuf::from("gump.toml")).unwrap();
    assert!(
        plan.archive_spill_path().is_file(),
        "expected archive spill file"
    );
    assert!(
        fs::metadata(plan.archive_spill_path()).unwrap().len() > 0,
        "spill must be non-empty"
    );
    drop(plan); // run_local builds its own spill independently
    let report = run_local(LocalRunOptions {
        workspace: ws.clone(),
        manifest_path: PathBuf::from("gump.toml"),
        state_root: Some(ws.join("state")),
    })
    .unwrap();
    assert_eq!(report.mode, "run");
    assert!(report.release_root.join("bin/hello").is_file());
    let _ = fs::remove_dir_all(ws);
}

#[test]
fn sealed_capsule_bytes_are_deterministic_with_fixed_rng() {
    let ws = tmp_workspace("det");
    write_fixture(&ws);
    let plan = local_parity_plan(&ws, &PathBuf::from("gump.toml")).unwrap();
    let capsule = CapsuleId::from_bytes(fixed_v7(1)).unwrap();
    let cluster = ClusterId::from_bytes(fixed_v7(2)).unwrap();

    let mut rng_a = ReplayRng::new(X01_RNG_SEED);
    let built_a = build_sealed_capsule(&plan, capsule, cluster, &mut rng_a).unwrap();
    let mut rng_b = ReplayRng::new(X01_RNG_SEED);
    let built_b = build_sealed_capsule(&plan, capsule, cluster, &mut rng_b).unwrap();

    assert_eq!(built_a.bytes, built_b.bytes);
    assert_eq!(built_a.archive_digest, built_b.archive_digest);
    assert_eq!(built_a.signature, built_b.signature);
    let _ = fs::remove_dir_all(ws);
}

#[test]
fn verified_local_run_after_sealed_build() {
    let ws = tmp_workspace("run");
    write_fixture(&ws);
    let plan = local_parity_plan(&ws, &PathBuf::from("gump.toml")).unwrap();
    let capsule = CapsuleId::from_bytes(fixed_v7(3)).unwrap();
    let cluster = ClusterId::from_bytes(fixed_v7(4)).unwrap();
    let mut rng = ReplayRng::new(X01_RNG_SEED);
    let built = build_sealed_capsule(&plan, capsule, cluster, &mut rng).unwrap();
    verify_sealed_capsule(&built).unwrap();

    let report = run_verified_sealed(&ws, Some(ws.join("state")), &plan, &built).unwrap();
    assert_eq!(report.mode, "test-sealed");
    assert_eq!(report.namespace, "ci");
    assert_eq!(report.app_id, "x01-hello");
    assert_eq!(report.command_vector, vec!["bin/hello".to_string()]);
    assert_eq!(report.exit_code, Some(0));
    let _ = fs::remove_dir_all(ws);
}

#[test]
fn tampered_signature_rejects_before_run() {
    let ws = tmp_workspace("tamper");
    write_fixture(&ws);
    let plan = local_parity_plan(&ws, &PathBuf::from("gump.toml")).unwrap();
    let capsule = CapsuleId::from_bytes(fixed_v7(5)).unwrap();
    let cluster = ClusterId::from_bytes(fixed_v7(6)).unwrap();
    let mut rng = ReplayRng::new(X01_RNG_SEED);
    let mut built = build_sealed_capsule(&plan, capsule, cluster, &mut rng).unwrap();
    built.signature[0] ^= 0xff;
    // Also flip the signature bytes embedded in the Capsule body (last 64 of sig segment).
    let n = built.bytes.len();
    assert!(n >= 64);
    built.bytes[n - 1] ^= 0xff;

    let err = verify_sealed_capsule(&built).unwrap_err();
    assert_eq!(err.kind(), gump_cli::CliErrorKind::Capsule);

    let run_err = run_verified_sealed(&ws, Some(ws.join("state")), &plan, &built).unwrap_err();
    assert_eq!(run_err.kind(), gump_cli::CliErrorKind::Capsule);
    assert!(
        !ws.join("state").join("apps").exists(),
        "must not materialize release roots after failed verification"
    );
    let _ = fs::remove_dir_all(ws);
}

fn write_fixture_with_secret_var(ws: &std::path::Path, with_local_literal: bool) {
    write_fixture(ws);
    let mut toml = fs::read_to_string(ws.join("gump.toml")).unwrap();
    toml.push_str(
        r#"
[runtime.variables.TOKEN]
source = "env:GUMP_X01_N007_TOKEN"
required = true
classification = "secret"
encoding = "utf8"
max_bytes = "4KiB"
inject = "env"
"#,
    );
    if with_local_literal {
        toml.push_str(
            r#"
[local.variables.TOKEN]
source = "literal:n007-e2e-canary-SECRET-7c2e"
"#,
        );
    }
    fs::write(ws.join("gump.toml"), toml).unwrap();
}

#[test]
fn sealed_packaging_keeps_canary_out_of_public_bytes() {
    let ws = tmp_workspace("n007-canary");
    write_fixture_with_secret_var(&ws, true);
    let plan = local_parity_plan(&ws, &PathBuf::from("gump.toml")).unwrap();
    let canary = b"n007-e2e-canary-SECRET-7c2e";
    let mut rng = ReplayRng::new(X01_RNG_SEED);
    let built = build_sealed_capsule(
        &plan,
        CapsuleId::from_bytes(fixed_v7(0x21)).unwrap(),
        ClusterId::from_bytes(fixed_v7(0x22)).unwrap(),
        &mut rng,
    )
    .unwrap();
    verify_sealed_capsule(&built).unwrap();

    // Public metadata is segment 1; scan whole Capsule for accidental plaintext leak
    // of the canary (ciphertext may coincidentally contain short patterns — canary is long).
    assert!(
        !built
            .bytes
            .windows(canary.len())
            .any(|w| w == canary.as_slice()),
        "canary must not appear in sealed Capsule bytes"
    );
    let _ = fs::remove_dir_all(ws);
}

#[test]
fn sealed_packaging_fails_closed_when_required_var_unset() {
    let ws = tmp_workspace("n007-unset");
    write_fixture_with_secret_var(&ws, false);
    let plan = local_parity_plan(&ws, &PathBuf::from("gump.toml")).unwrap();
    let mut rng = ReplayRng::new(X01_RNG_SEED);
    let err = build_sealed_capsule(
        &plan,
        CapsuleId::from_bytes(fixed_v7(0x23)).unwrap(),
        ClusterId::from_bytes(fixed_v7(0x24)).unwrap(),
        &mut rng,
    )
    .unwrap_err();
    assert_eq!(err.kind(), gump_cli::CliErrorKind::Policy);
    assert!(
        !err.to_string().contains("n007-e2e-canary"),
        "errors must not echo secret values"
    );
    let _ = fs::remove_dir_all(ws);
}

#[test]
fn unsealed_run_and_sealed_path_share_command_vector() {
    let ws = tmp_workspace("parity");
    write_fixture(&ws);
    let unsealed = run_local(LocalRunOptions {
        workspace: ws.clone(),
        manifest_path: PathBuf::from("gump.toml"),
        state_root: Some(ws.join("state-run")),
    })
    .unwrap();

    let plan = local_parity_plan(&ws, &PathBuf::from("gump.toml")).unwrap();
    let mut rng = ReplayRng::new(X01_RNG_SEED);
    let built = build_sealed_capsule(
        &plan,
        CapsuleId::from_bytes(fixed_v7(7)).unwrap(),
        ClusterId::from_bytes(fixed_v7(8)).unwrap(),
        &mut rng,
    )
    .unwrap();
    let sealed = run_verified_sealed(&ws, Some(ws.join("state-sealed")), &plan, &built).unwrap();

    assert_eq!(unsealed.command_vector, sealed.command_vector);
    assert_eq!(unsealed.namespace, sealed.namespace);
    assert_eq!(unsealed.app_id, sealed.app_id);
    assert_eq!(unsealed.exit_code, Some(0));
    assert_eq!(sealed.exit_code, Some(0));
    let _ = fs::remove_dir_all(ws);
}
