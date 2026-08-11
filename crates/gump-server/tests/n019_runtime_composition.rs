//! Committed desired binding → verified materialization → schedule → secret
//! admission → driver start → telemetry.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use gump_cli::{build_sealed_capsule_for_cluster, local_parity_plan};
use gump_connectors::{FakeObjectStore, RuntimeObjectStore};
use gump_crypto::{
    RecoverySecret, SignerEnrollment, SignerTrustPolicy, generate_signing_key, verifying_key,
};
use gump_memory::{RaftCommand, RaftResponse};
use gump_server::compose::{InitOptions, ProductRuntime};
use gump_server::deploy_txn::{
    DeployTxnOutcome, DeployTxnRequest, DesiredCapsuleBindingV1, run_verified_deploy_txn,
};
use gump_types::{CapsuleId, ClusterId};
use rand_core::{TryCryptoRng, TryRng};

struct TestRng(u64);
impl TryRng for TestRng {
    type Error = core::convert::Infallible;
    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        Ok((self.0 >> 32) as u32)
    }
    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok((u64::from(self.try_next_u32()?) << 32) | u64::from(self.try_next_u32()?))
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Self::Error> {
        for byte in dest {
            *byte = self.try_next_u32()? as u8;
        }
        Ok(())
    }
}
impl TryCryptoRng for TestRng {}

#[test]
fn desired_capsule_is_started_and_stdout_reaches_telemetry() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(workspace.join("bin")).unwrap();
    let executable = workspace.join("bin/app");
    fs::write(
        &executable,
        "#!/bin/sh\necho run >> run-count\necho runtime-composed\nsleep 0.1\n",
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        workspace.join("gump.toml"),
        r#"
schema = "gump/1"
[app]
id = "runtime-composition"
namespace = "ci"
[workload]
lifetime = "continuous"
coordination = "independent"
success = "never"
[package]
root = "."
include = ["bin/app"]
exclude = []
[runtime]
driver = "native"
command = ["./bin/app"]
workdir = "."
[telemetry]
protocol = "ratatouille/0.1"
format = "ndjson"
filter = "app/*"
"#,
    )
    .unwrap();

    let cluster_id = ClusterId::new();
    let capsule_id = CapsuleId::new();
    let mut rng = TestRng(19);
    let signing = generate_signing_key(&mut rng);
    let verifying = verifying_key(&signing);
    let mut trust = SignerTrustPolicy::new();
    trust
        .enroll(SignerEnrollment {
            public_key: verifying,
            namespaces: BTreeSet::from(["ci".into()]),
            expires_at_ms: None,
            capabilities: BTreeSet::new(),
        })
        .unwrap();
    let runtime = ProductRuntime::init_with_state_root(
        InitOptions {
            cluster_id: Some(cluster_id),
            object_store: Some(RuntimeObjectStore::Memory(FakeObjectStore::new())),
            signer_trust: trust,
            ..InitOptions::default()
        },
        temp.path().join("state"),
    )
    .unwrap();
    let secret = RecoverySecret::from_bytes([0x29; 32]);
    let cluster_public = {
        let mut custody = runtime.local_api.custody.as_ref().unwrap().lock().unwrap();
        custody.activate_software_1of1(&secret, "test-key").unwrap();
        gump_crypto::ClusterX25519Public(custody.cluster_public().unwrap())
    };
    let plan = local_parity_plan(&workspace, &PathBuf::from("gump.toml")).unwrap();
    let built = build_sealed_capsule_for_cluster(
        &plan,
        capsule_id,
        cluster_id,
        &signing,
        &cluster_public,
        "test-key",
        &mut rng,
    )
    .unwrap();
    let digest = *blake3::hash(&built.bytes).as_bytes();
    let operation_id = CapsuleId::new();
    let capsule_bytes = built.bytes;
    let mismatch_operation_id = CapsuleId::new();
    let mismatch = {
        let mut store = runtime
            .local_api
            .object_store
            .as_ref()
            .unwrap()
            .lock()
            .unwrap();
        let mut orphans = runtime.local_api.deploy_orphans.lock().unwrap();
        run_verified_deploy_txn(
            &mut *store,
            &runtime.local_api.signer_trust,
            runtime.local_api.memory_cluster.as_ref().unwrap(),
            &mut orphans,
            DeployTxnRequest {
                operation_id: *mismatch_operation_id.as_bytes(),
                operation_id_display: mismatch_operation_id.to_hyphenated(),
                namespace: "ci".into(),
                app: "wrong-app".into(),
                expected_generation: 0,
                content_digest: digest,
                capsule_bytes: Some(capsule_bytes.clone()),
                cluster_id,
                capsule_id,
            },
            1,
        )
    };
    assert!(matches!(
        mismatch,
        DeployTxnOutcome::Failed {
            phase: gump_connectors::DeployPhase::LocalValidation,
            ..
        }
    ));

    let outcome = {
        let mut store = runtime
            .local_api
            .object_store
            .as_ref()
            .unwrap()
            .lock()
            .unwrap();
        let mut orphans = runtime.local_api.deploy_orphans.lock().unwrap();
        run_verified_deploy_txn(
            &mut *store,
            &runtime.local_api.signer_trust,
            runtime.local_api.memory_cluster.as_ref().unwrap(),
            &mut orphans,
            DeployTxnRequest {
                operation_id: *operation_id.as_bytes(),
                operation_id_display: operation_id.to_hyphenated(),
                namespace: "ci".into(),
                app: "runtime-composition".into(),
                expected_generation: 0,
                content_digest: digest,
                capsule_bytes: Some(capsule_bytes),
                cluster_id,
                capsule_id,
            },
            1,
        )
    };
    assert!(matches!(outcome, DeployTxnOutcome::Success { .. }));

    let status = runtime
        .execution
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .reconcile(
            runtime.local_api.memory_cluster.as_ref().unwrap(),
            runtime.local_api.object_store.as_ref().unwrap(),
            10,
        )
        .unwrap();
    assert_eq!(status.placements, 1);
    let reads_after_first_reconcile = {
        let store = runtime
            .local_api
            .object_store
            .as_ref()
            .unwrap()
            .lock()
            .unwrap();
        match &*store {
            RuntimeObjectStore::Memory(store) => store.read_call_counts(),
            RuntimeObjectStore::S3(_) => unreachable!("test runtime uses memory object store"),
        }
    };
    let unchanged = runtime
        .execution
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .reconcile(
            runtime.local_api.memory_cluster.as_ref().unwrap(),
            runtime.local_api.object_store.as_ref().unwrap(),
            11,
        )
        .unwrap();
    assert_eq!(unchanged.placements, 1);
    let reads_after_unchanged_reconcile = {
        let store = runtime
            .local_api
            .object_store
            .as_ref()
            .unwrap()
            .lock()
            .unwrap();
        match &*store {
            RuntimeObjectStore::Memory(store) => store.read_call_counts(),
            RuntimeObjectStore::S3(_) => unreachable!("test runtime uses memory object store"),
        }
    };
    assert_eq!(
        reads_after_unchanged_reconcile, reads_after_first_reconcile,
        "unchanged reconciliation must perform zero object-store reads"
    );
    let mut observed = false;
    for _ in 0..40 {
        let telemetry = runtime
            .local_api
            .telemetry
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .query(Some("app/stdout"), 32);
        observed = telemetry.events.iter().any(|event| {
            matches!(
                event,
                gump_telemetry::TelemetryEventView::Record { text: Some(text), .. }
                    if text.contains("runtime-composed")
            )
        });
        if observed {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(observed);
    std::thread::sleep(std::time::Duration::from_millis(350));
    let run_count = fs::read_to_string(
        temp.path()
            .join("state/apps")
            .join(capsule_id.to_hyphenated())
            .join("run-count"),
    )
    .unwrap();
    assert_eq!(
        run_count.lines().count(),
        1,
        "finite intent ran more than once"
    );

    let missing_capsule = CapsuleId::new();
    let missing_digest = [0xabu8; 32];
    let missing_payload = serde_json::to_vec(&DesiredCapsuleBindingV1 {
        schema: "gump.desired-capsule/1".into(),
        operation_id: "missing-capsule-backoff".into(),
        capsule_id: missing_capsule.to_hyphenated(),
    })
    .unwrap();
    assert!(matches!(
        runtime
            .local_api
            .memory_cluster
            .as_ref()
            .unwrap()
            .client_write(RaftCommand::PutDesired {
                namespace: "ci".into(),
                app: "runtime-composition".into(),
                expected_generation: 1,
                payload: missing_payload,
                content_digest: missing_digest,
            })
            .unwrap(),
        RaftResponse::Applied(_)
    ));
    let execution = runtime.execution.as_ref().unwrap();
    assert!(
        execution
            .lock()
            .unwrap()
            .reconcile(
                runtime.local_api.memory_cluster.as_ref().unwrap(),
                runtime.local_api.object_store.as_ref().unwrap(),
                10_000,
            )
            .is_err()
    );
    let reads_after_failed_fetch = {
        let store = runtime
            .local_api
            .object_store
            .as_ref()
            .unwrap()
            .lock()
            .unwrap();
        match &*store {
            RuntimeObjectStore::Memory(store) => store.read_call_counts(),
            RuntimeObjectStore::S3(_) => unreachable!("test runtime uses memory object store"),
        }
    };
    assert!(
        execution
            .lock()
            .unwrap()
            .reconcile(
                runtime.local_api.memory_cluster.as_ref().unwrap(),
                runtime.local_api.object_store.as_ref().unwrap(),
                10_001,
            )
            .is_err()
    );
    let reads_during_backoff = {
        let store = runtime
            .local_api
            .object_store
            .as_ref()
            .unwrap()
            .lock()
            .unwrap();
        match &*store {
            RuntimeObjectStore::Memory(store) => store.read_call_counts(),
            RuntimeObjectStore::S3(_) => unreachable!("test runtime uses memory object store"),
        }
    };
    assert_eq!(
        reads_during_backoff, reads_after_failed_fetch,
        "failed Capsule fetches must not retry on every 250 ms reconciliation"
    );
}
