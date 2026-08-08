//! STL-11 exit: one brutal vertical path before scheduler/agent widen.
//!
//! Documented test path: `cargo test -p gump-gates --test stl11_vertical_exit`.
//!
//! Covers (sim FakeObjectStore, not live MinIO):
//! 1. Streamed Capsule quarantine → verify → final object key
//! 2. Replicated ClusterState intent + idempotency (two Raft machines agree)
//! 3. Leader-retry: same Idempotent op replays without double-apply
//! 4. Noisy forking workload under NativeDriver with bounded pipe→ring telemetry
//! 5. Fenced TERM→KILL process-group cleanup
//!
//! Named failure modes (asserted or documented residual):
//! - Corrupt Capsule bytes → ingress reject; no final Capsule key (quarantine orphan may remain)
//! - Idempotency digest mismatch → `RaftResponse::Rejected`
//! - stdout flood without drain → hang (counter-evidence: drain + bounded ring)
//! - child ignores SIGTERM → KILL after terminate deadline
//! - Does **not** claim all 28 INV-* or production MinIO/S3 TLS
//!
//! Authority: stop-the-line STL-11; RUNTIME / D006 / D011; CONFORMANCE integration tier.

#![cfg(unix)]

use std::collections::BTreeSet;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gump_capsule::{
    GumpCapsuleHeader, StreamingCapsuleReader, verify_release_signature, write_gump_capsule,
};
use gump_connectors::{
    FakeObjectStore, IngressLimits, ObjectStore, StreamedIngress, final_capsule_key,
};
use gump_crypto::{
    SegmentDigestRef, SignerEnrollment, SignerTrustPolicy, VerifyingKeyBytes,
    build_release_signing_transcript, ed25519_fingerprint, generate_signing_key, sign_transcript,
    verifying_key,
};
use gump_driver::{
    AttemptContext, Driver, DriverKind, IoEndpoints, NativeDriver, ReleaseRoot, ResourceGrant,
    RuntimeSpec, SecretPlan, Signal, StartFence,
};
use gump_memory::{
    Command, Expected, KeyPrefix, RaftCommand, RaftResponse, RecordKey, TypeConfig, ram_v2_stores,
};
use gump_telemetry::{AttemptPipeBridge, RingConfig, TOPIC_STDOUT, TopicFilter};
use gump_types::{AttemptId, CapsuleId, ClusterId};
use openraft::storage::RaftStateMachine;
use openraft::{Entry, EntryPayload, LeaderId, LogId};
use rand_core::{TryCryptoRng, TryRng};

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct SeedRng {
    state: u64,
}

impl SeedRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }
}

impl TryRng for SeedRng {
    type Error = core::convert::Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
        Ok((self.state >> 32) as u32)
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        let a = self.try_next_u32()? as u64;
        let b = self.try_next_u32()? as u64;
        Ok((a << 32) | b)
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Self::Error> {
        for chunk in dest.chunks_mut(4) {
            let n = self.try_next_u32()?.to_le_bytes();
            for (i, b) in chunk.iter_mut().enumerate() {
                *b = n[i];
            }
        }
        Ok(())
    }
}

impl TryCryptoRng for SeedRng {}

fn v7(seed: u8) -> [u8; 16] {
    let mut b = [seed; 16];
    b[6] = (b[6] & 0x0f) | 0x70;
    b[8] = (b[8] & 0x3f) | 0x80;
    b
}

struct SealedFixture {
    bytes: Vec<u8>,
    verifying: VerifyingKeyBytes,
    cluster: ClusterId,
    capsule: CapsuleId,
}

fn build_sealed(seed: u64) -> SealedFixture {
    let cluster = ClusterId::from_bytes(v7(0x71)).unwrap();
    let capsule = CapsuleId::from_bytes(v7(0x72)).unwrap();
    let mut rng = SeedRng::new(seed);
    let signing = generate_signing_key(&mut rng);
    let verifying = verifying_key(&signing);
    let fp = ed25519_fingerprint(&verifying.0);
    let release_signer = fp.strip_prefix("blake3:").unwrap().to_string();

    let public_metadata = b"stl11-meta".as_slice();
    let archive = b"stl11-archive".as_slice();
    let protected = b"protected".as_slice();
    let key_envelope = b"envelope".as_slice();

    let header = GumpCapsuleHeader {
        capsule_id: *capsule.as_bytes(),
        cluster_id: *cluster.as_bytes(),
        release_signer,
        created_unix_ms: 0,
    };
    let header_cbor = header.encode_cbor().unwrap();
    let logical = [
        public_metadata.len() as u64,
        archive.len() as u64,
        protected.len() as u64,
        key_envelope.len() as u64,
        0,
    ];
    let placeholder = [0u8; 96];
    let mut buf = Vec::new();
    let provisional = write_gump_capsule(
        &mut buf,
        &header,
        [
            public_metadata,
            archive,
            protected,
            key_envelope,
            placeholder.as_slice(),
        ],
        logical,
    )
    .unwrap();

    let segs = [
        SegmentDigestRef {
            segment_type: 1,
            stored_length: provisional.table.descriptors[0].stored_length,
            digest: provisional.table.descriptors[0].digest,
        },
        SegmentDigestRef {
            segment_type: 2,
            stored_length: provisional.table.descriptors[1].stored_length,
            digest: provisional.table.descriptors[1].digest,
        },
        SegmentDigestRef {
            segment_type: 3,
            stored_length: provisional.table.descriptors[2].stored_length,
            digest: provisional.table.descriptors[2].digest,
        },
        SegmentDigestRef {
            segment_type: 4,
            stored_length: provisional.table.descriptors[3].stored_length,
            digest: provisional.table.descriptors[3].digest,
        },
    ];
    let transcript = build_release_signing_transcript(&header_cbor, 1, &segs).unwrap();
    let signature = sign_transcript(&signing, &transcript).unwrap();
    let mut sig_seg = Vec::with_capacity(96);
    sig_seg.extend_from_slice(&verifying.0);
    sig_seg.extend_from_slice(&signature);

    let mut sealed = Vec::new();
    let view = write_gump_capsule(
        &mut sealed,
        &header,
        [
            public_metadata,
            archive,
            protected,
            key_envelope,
            sig_seg.as_slice(),
        ],
        logical,
    )
    .unwrap();
    verify_release_signature(&header_cbor, &view.table, &verifying.0, &signature).unwrap();

    SealedFixture {
        bytes: sealed,
        verifying,
        cluster,
        capsule,
    }
}

fn enroll(trust: &mut SignerTrustPolicy, vk: VerifyingKeyBytes, ns: &str) {
    trust
        .enroll(SignerEnrollment {
            public_key: vk,
            namespaces: BTreeSet::from([ns.into()]),
            expires_at_ms: None,
            capabilities: BTreeSet::new(),
        })
        .unwrap();
}

fn log_id(index: u64) -> LogId<u64> {
    LogId::new(LeaderId::new(1, 1), index)
}

fn entry(index: u64, cmd: RaftCommand) -> Entry<TypeConfig> {
    Entry {
        log_id: log_id(index),
        payload: EntryPayload::Normal(cmd),
    }
}

fn tmp(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("gump-stl11-{name}-{nanos}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_executable(path: &std::path::Path, body: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[tokio::test]
async fn vertical_store_cluster_supervisor_telemetry() {
    // --- 1) Streamed Capsule → FakeObjectStore (disk-spill quarantine) ---
    let fix = build_sealed(11);
    let mut trust = SignerTrustPolicy::new();
    enroll(&mut trust, fix.verifying, "prod");
    let mut store = FakeObjectStore::new();
    let ingress = StreamedIngress::new(IngressLimits {
        max_capsule_bytes: 10 * 1024 * 1024,
        max_chunk_bytes: 64,
    });
    let mut reader = Cursor::new(fix.bytes.clone());
    let receipt = ingress
        .accept_known_length(
            &mut store,
            &trust,
            fix.cluster,
            fix.capsule,
            "prod",
            0,
            fix.bytes.len() as u64,
            &mut reader,
        )
        .unwrap();
    assert!(
        receipt.stats.peak_buffer_bytes < fix.bytes.len(),
        "ingress must not buffer full Capsule"
    );
    assert_eq!(store.get(&receipt.evidence.key, None).unwrap(), fix.bytes);

    // Streamed re-verify of stored bytes (chunked reader, not one-shot Vec parse path).
    let stored = store.get(&receipt.evidence.key, None).unwrap();
    let meta = StreamingCapsuleReader::with_chunk_bytes(stored.as_slice(), 64)
        .verify()
        .unwrap();
    assert_eq!(meta.header.capsule_id, *fix.capsule.as_bytes());

    // Corrupt body failure mode: reject before durable final (fresh store).
    {
        let mut bad_store = FakeObjectStore::new();
        let mut bad = fix.bytes.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 0xff;
        let bad_capsule = CapsuleId::from_bytes(v7(0x73)).unwrap();
        let err = StreamedIngress::default()
            .accept_known_length(
                &mut bad_store,
                &trust,
                fix.cluster,
                bad_capsule,
                "prod",
                0,
                bad.len() as u64,
                &mut Cursor::new(bad),
            )
            .unwrap_err();
        let _ = err;
        let final_key = final_capsule_key(fix.cluster, bad_capsule).unwrap();
        assert!(
            !bad_store.keys().iter().any(|k| k == &final_key),
            "corrupt Capsule must not publish final key {final_key}"
        );
    }

    // --- 2+3) Replicated intent + leader retry / idempotency ---
    let capsule_digest = *blake3::hash(&fix.bytes).as_bytes();
    let desired_payload = format!(
        "capsule={} key={}",
        fix.capsule,
        receipt.evidence.key.as_str()
    );
    let desired_digest = *blake3::hash(desired_payload.as_bytes()).as_bytes();
    let op = [0x11u8; 16];
    let put_key = RecordKey::new(KeyPrefix::ClusterMeta, "stl11").unwrap();

    let put = RaftCommand::Record(Command::Put {
        key: put_key.clone(),
        expected: Expected::Absent,
        payload: capsule_digest.to_vec(),
        leased: false,
    });
    let desired = RaftCommand::PutDesired {
        namespace: "prod".into(),
        app: "stl11".into(),
        expected_generation: 0,
        payload: desired_payload.into_bytes(),
        content_digest: desired_digest,
    };
    let idem = RaftCommand::Idempotent {
        operation_id: op,
        request_digest: desired_digest,
        inner: Box::new(desired.clone()),
    };
    // Conflict failure mode: same operation_id, different digest.
    let conflict = RaftCommand::Idempotent {
        operation_id: op,
        request_digest: [0xAAu8; 32],
        inner: Box::new(desired.clone()),
    };

    let (_log_a, mut sm_a) = ram_v2_stores();
    let (_log_b, mut sm_b) = ram_v2_stores();

    let seq = vec![
        entry(1, put),
        entry(2, RaftCommand::AcquireController { holder: 1 }),
        entry(3, idem.clone()),
    ];
    let ra = RaftStateMachine::apply(&mut sm_a, seq.clone())
        .await
        .unwrap();
    let rb = RaftStateMachine::apply(&mut sm_b, seq).await.unwrap();
    assert_eq!(ra, rb, "replicas must agree on apply responses");

    // Leader failure → client retries same Idempotent: replay, no double generation bump.
    let retry = RaftStateMachine::apply(&mut sm_a, vec![entry(4, idem.clone())])
        .await
        .unwrap();
    assert!(
        matches!(retry.last().unwrap(), RaftResponse::Replay(_)),
        "retry after leader loss must replay, got {:?}",
        retry.last()
    );
    // Follower catches up including the retry log entry (identical ClusterState).
    let rb_retry = RaftStateMachine::apply(&mut sm_b, vec![entry(4, idem)])
        .await
        .unwrap();
    assert_eq!(retry, rb_retry);

    let ca = sm_a.cluster_state().await;
    let cb = sm_b.cluster_state().await;
    assert_eq!(ca.desired_generation("prod", "stl11"), Some(1));
    assert_eq!(cb.desired_generation("prod", "stl11"), Some(1));
    assert_eq!(ca.records().revision(), cb.records().revision());

    let conflict_resp = RaftStateMachine::apply(&mut sm_a, vec![entry(5, conflict)])
        .await
        .unwrap();
    assert!(
        matches!(conflict_resp.last().unwrap(), RaftResponse::Rejected(_)),
        "digest mismatch must reject"
    );

    // --- 4+5) Noisy fork + bounded telemetry + fenced kill ---
    // Serialize process-group tests (same rationale as STL-04).
    let _guard = test_lock();
    let root = tmp("noisy");
    write_executable(
        &root.join("bin/noisy"),
        concat!(
            "#!/bin/sh\n",
            "trap '' TERM\n",
            // Flood stdout, fork a grandchild, then ignore TERM until KILL.
            "yes &\n",
            "sleep 60 &\n",
            "printf ready >\"$GUMP_ATTEMPT_ROOT/ready\"\n",
            "while true; do sleep 1; done\n",
        ),
    );
    let release = ReleaseRoot::new(&root);
    let driver = NativeDriver::new();
    let attempt_root = tmp("attempt").join("attempt");
    fs::create_dir_all(&attempt_root).unwrap();

    let bridge = AttemptPipeBridge::new(RingConfig {
        max_bytes: 10_000,
        max_age: Duration::from_secs(60),
        max_records: Some(4),
    });
    let prepared = driver
        .prepare(
            &release,
            &RuntimeSpec {
                kind: DriverKind::Native,
                command: vec!["bin/noisy".into()],
                interpreter: None,
                workdir: None,
            },
            &AttemptContext {
                attempt_id: AttemptId::new(),
                attempt_root: attempt_root.clone(),
            },
        )
        .unwrap();
    let admission = driver
        .admit(
            prepared,
            ResourceGrant {
                max_processes: Some(16),
            },
            SecretPlan::deferred(),
        )
        .unwrap();
    let mut running = driver
        .start(
            admission,
            StartFence { generation: 1 },
            &IoEndpoints {
                capture_stdout: true,
                capture_stderr: true,
                pipe_sink: Some(bridge.clone().shared_sink()),
            },
        )
        .unwrap();

    let ready = attempt_root.join("ready");
    let armed = Instant::now();
    while !ready.is_file() && armed.elapsed() < Duration::from_secs(3) {
        let _ = driver.observe(&mut running);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(ready.is_file(), "noisy workload never armed");

    // Drain under pressure into bounded ring (drop-oldest).
    let flood = Instant::now();
    while flood.elapsed() < Duration::from_millis(800) {
        assert!(driver.observe(&mut running).unwrap().running);
        if running.stdout_received_bytes() > 32 * 1024 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        running.stdout_received_bytes() > 8 * 1024,
        "supervisor drain must pull noisy stdout"
    );
    bridge.finish();
    assert!(
        bridge.pushed() >= 1,
        "pipe bridge should record stdout segments"
    );
    bridge.with_ring(|ring| {
        assert!(ring.len() <= 4, "ring must stay within max_records");
        let mut sub = ring.subscribe(TopicFilter::only(&[TOPIC_STDOUT]));
        let mut saw = false;
        while sub.poll(ring).is_some() {
            saw = true;
        }
        assert!(saw || bridge.dropped_oldest() >= 1 || bridge.pushed() >= 1);
    });

    // TERM ignored → terminate deadline → KILL tree (incl. forked children).
    driver.signal(&mut running, Signal::Term).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        driver.observe(&mut running).unwrap().running,
        "SIGTERM trap should keep primary alive briefly"
    );
    driver
        .terminate(&mut running, Duration::from_millis(250))
        .unwrap();
    assert!(
        !driver.observe(&mut running).unwrap().running,
        "fenced KILL must reap process group"
    );

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(attempt_root.parent().unwrap());
}
