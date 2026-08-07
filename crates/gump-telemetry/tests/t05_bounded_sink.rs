//! STL-09: bounded callback sink + pipe-drain bridge (D011 drop-oldest).

use std::sync::Arc;
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
