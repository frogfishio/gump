//! STL-09: bounded callback sink + pipe-drain bridge (D011 drop-oldest).

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use gump_driver::PipeChunkSink;
use gump_telemetry::{
    AttemptPipeBridge, BoundedCallbackAdapter, CanonicalIdentity, RecordOutcome, RingConfig,
    TOPIC_STDOUT,
};
use gump_types::{AttemptId, CapsuleId, ClusterId, ExecutionId, Label, NodeId, UnitId, WorkloadId};

fn v7(seed: u8) -> [u8; 16] {
    let mut b = [seed; 16];
    b[6] = (b[6] & 0x0f) | 0x70;
    b[8] = (b[8] & 0x3f) | 0x80;
    b
}

fn sample_identity() -> CanonicalIdentity {
    CanonicalIdentity {
        cluster_id: ClusterId::from_bytes(v7(1)).unwrap(),
        namespace: Label::parse("default").unwrap(),
        app_id: Label::parse("accounts").unwrap(),
        workload_id: WorkloadId::from_bytes(v7(2)).unwrap(),
        release_id: CapsuleId::from_bytes(v7(3)).unwrap(),
        execution_id: ExecutionId::from_bytes(v7(4)).unwrap(),
        unit_id: UnitId::from_bytes(v7(5)).unwrap(),
        role: Some(Label::parse("worker").unwrap()),
        rank: Some(0),
        attempt_id: AttemptId::from_bytes(v7(6)).unwrap(),
        node_id: NodeId::from_bytes(v7(7)).unwrap(),
        agent_incarnation: 9,
    }
}

#[test]
fn bounded_callback_overflow_drops_oldest() {
    let mut adapter = BoundedCallbackAdapter::with_max_records(sample_identity(), 2);
    assert_eq!(
        adapter.ingest_line("app/stdout one"),
        RecordOutcome::Accepted
    );
    assert_eq!(
        adapter.ingest_line("app/stdout two"),
        RecordOutcome::Accepted
    );
    assert_eq!(
        adapter.ingest_line("app/stdout three"),
        RecordOutcome::AcceptedDropOldest
    );
    assert_eq!(adapter.dropped_oldest(), 1);
    assert_eq!(adapter.records().len(), 2);
    assert_eq!(adapter.records().front().unwrap().message, "two");
    assert_eq!(adapter.records().back().unwrap().message, "three");
}

#[test]
fn bounded_callback_charges_large_producer_exactly() {
    // ~60 KiB producer.app must be charged at full length (not ~256B fudge).
    let producer = "p".repeat(60 * 1024);
    let line = format!(r#"{{"topic":"app/stdout","msg":"hi","src":{{"app":"{producer}"}}}}"#);
    assert!(line.len() <= gump_telemetry::MAX_RECORD_BYTES);

    let mut adapter = BoundedCallbackAdapter::new(sample_identity());
    assert_eq!(adapter.ingest_line(&line), RecordOutcome::Accepted);

    let expected_min = producer.len() + "app/stdout".len() + "hi".len();
    assert!(
        adapter.total_bytes() >= expected_min,
        "charged {} < expected_min {expected_min}",
        adapter.total_bytes()
    );
    // Must not under-charge by ignoring producer (old bug: topic+message+256).
    assert!(
        adapter.total_bytes() > 256 + "app/stdout".len() + "hi".len(),
        "producer bytes were not charged"
    );
}

#[test]
fn bounded_callback_rejects_oversized_on_empty_queue() {
    let mut adapter = BoundedCallbackAdapter::with_config(
        sample_identity(),
        RingConfig {
            max_bytes: 64,
            max_age: Duration::from_secs(60),
            max_records: None,
        },
    );
    let line = format!("app/stdout {}", "x".repeat(200));
    assert_eq!(adapter.ingest_line(&line), RecordOutcome::RejectedOversize);
    assert_eq!(adapter.records().len(), 0);
    assert_eq!(adapter.rejected_oversize(), 1);
    assert_eq!(adapter.accepted(), 0);
}

#[test]
fn bounded_callback_max_age_evicts_stale() {
    let mut adapter = BoundedCallbackAdapter::with_config(
        sample_identity(),
        RingConfig {
            max_bytes: 1_000_000,
            max_age: Duration::from_millis(40),
            max_records: None,
        },
    );
    assert_eq!(
        adapter.ingest_line("app/stdout one"),
        RecordOutcome::Accepted
    );
    thread::sleep(Duration::from_millis(80));
    assert_eq!(
        adapter.ingest_line("app/stdout two"),
        RecordOutcome::Accepted
    );
    assert_eq!(adapter.records().len(), 1);
    assert_eq!(adapter.records().front().unwrap().message, "two");
    assert!(adapter.dropped_oldest() >= 1);
}

#[test]
fn pipe_bridge_on_chunk_does_not_block_when_ring_locked() {
    let bridge = AttemptPipeBridge::new(RingConfig {
        max_bytes: 10_000,
        max_age: Duration::from_secs(60),
        max_records: Some(8),
    });
    let sink: Arc<dyn PipeChunkSink> = bridge.clone().shared_sink();

    let started = Arc::new(std::sync::Barrier::new(2));
    let started2 = Arc::clone(&started);
    let bridge_hold = bridge.clone();
    let holder = thread::spawn(move || {
        bridge_hold.with_ring(|_ring| {
            started2.wait();
            thread::sleep(Duration::from_millis(150));
        });
    });

    started.wait();
    // While with_ring holds the mutex, on_chunk must return promptly and count drops.
    sink.on_chunk(gump_driver::StreamKind::Stdout, b"noisy\n");
    assert!(bridge.lock_busy_drops() >= 1);
    holder.join().expect("holder");
}

#[test]
fn pipe_bridge_segments_into_local_ring_and_drops_oldest() {
    let bridge = AttemptPipeBridge::new(RingConfig {
        max_bytes: 10_000,
        max_age: Duration::from_secs(60),
        max_records: Some(2),
    });
    let sink: Arc<dyn PipeChunkSink> = bridge.clone().shared_sink();

    sink.on_chunk(gump_driver::StreamKind::Stdout, b"line-a\n");
    sink.on_chunk(gump_driver::StreamKind::Stdout, b"line-b\n");
    sink.on_chunk(gump_driver::StreamKind::Stdout, b"line-c\n");
    bridge.finish();

    assert!(bridge.dropped_oldest() >= 1);
    assert!(bridge.pushed() >= 2);
    bridge.with_ring(|ring| {
        assert!(ring.len() <= 2);
        let mut sub = ring.subscribe(gump_telemetry::TopicFilter::only(&[TOPIC_STDOUT]));
        let mut msgs = Vec::new();
        while let Some(ev) = sub.poll(ring) {
            if let gump_telemetry::RingEvent::Record(r) = ev {
                msgs.push(String::from_utf8_lossy(&r.bytes).into_owned());
            }
        }
        assert!(msgs.iter().any(|m| m.contains("line-c")));
        assert!(!msgs.iter().any(|m| m.contains("line-a")));
    });
}
