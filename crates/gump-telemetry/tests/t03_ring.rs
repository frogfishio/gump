//! T03 exit evidence: gap, backpressure, and window tests for LocalRing.
//!
//! Authority: docs/v1/DELIVERY.md T03, RUNTIME.md §14, TELEMETRY.md §7/§12, D011.

use std::time::{Duration, Instant};

use gump_telemetry::{
    ChunkFlags, DEFAULT_RING_MAX_AGE, DEFAULT_RING_MAX_BYTES, GapReason, LocalRing, RingConfig,
    RingEvent, StreamDrain, StreamKind, StreamRecord, TOPIC_STDOUT, TopicFilter,
};

fn rec(topic: &'static str, seq: u64, bytes: &[u8]) -> StreamRecord {
    StreamRecord {
        topic,
        stream_sequence: seq,
        flags: ChunkFlags::BEGIN | ChunkFlags::END,
        bytes: bytes.to_vec(),
        utf8_hint: true,
        receive_offset: seq.saturating_mul(8),
    }
}

#[test]
fn defaults_match_d011() {
    assert_eq!(DEFAULT_RING_MAX_BYTES, 8 * 1024 * 1024);
    assert_eq!(DEFAULT_RING_MAX_AGE, Duration::from_secs(30));
}

#[test]
fn window_replay_then_live_tail() {
    let mut ring = LocalRing::new(RingConfig {
        max_bytes: 1024,
        max_age: Duration::from_secs(60),
        max_records: Some(100),
    });
    let t0 = Instant::now();
    ring.push(rec(TOPIC_STDOUT, 0, b"a\n"), t0);
    ring.push(rec(TOPIC_STDOUT, 1, b"b\n"), t0);

    let mut sub = ring.subscribe(TopicFilter::all());
    assert!(matches!(sub.poll(&ring), Some(RingEvent::Record(r)) if r.bytes == b"a\n"));
    assert!(matches!(sub.poll(&ring), Some(RingEvent::Record(r)) if r.bytes == b"b\n"));
    assert!(sub.poll(&ring).is_none());

    ring.push(rec(TOPIC_STDOUT, 2, b"c\n"), t0);
    assert!(matches!(sub.poll(&ring), Some(RingEvent::Record(r)) if r.bytes == b"c\n"));
    assert!(sub.poll(&ring).is_none());
}

#[test]
fn backpressure_overflow_emits_subscriber_lag_gap() {
    let mut ring = LocalRing::new(RingConfig {
        max_bytes: 10_000,
        max_age: Duration::from_secs(60),
        max_records: Some(2),
    });
    let t0 = Instant::now();
    ring.push(rec(TOPIC_STDOUT, 0, b"0\n"), t0);
    ring.push(rec(TOPIC_STDOUT, 1, b"1\n"), t0);

    let mut sub = ring.subscribe(TopicFilter::only(&[TOPIC_STDOUT]));
    let _ = sub.poll(&ring); // record 0
    let _ = sub.poll(&ring); // record 1
    assert!(sub.poll(&ring).is_none());

    // Overflow drops oldest while subscriber is caught up at end.
    ring.push(rec(TOPIC_STDOUT, 2, b"2\n"), t0);
    ring.push(rec(TOPIC_STDOUT, 3, b"3\n"), t0);
    assert!(ring.dropped_oldest() >= 2);

    match sub.poll(&ring) {
        Some(RingEvent::Gap(g)) => {
            assert_eq!(g.reason, GapReason::SubscriberLag);
            assert_eq!(g.topic, TOPIC_STDOUT);
        }
        other => panic!("expected lag gap, got {other:?}"),
    }
    // After gap, window replay from clamped cursor yields remaining records.
    let mut saw = Vec::new();
    while let Some(ev) = sub.poll(&ring) {
        if let RingEvent::Record(r) = ev {
            saw.push(r.stream_sequence);
        }
    }
    assert!(saw.contains(&2) || saw.contains(&3));
}

#[test]
fn sequence_hole_in_window_emits_overflow_gap() {
    let mut ring = LocalRing::new(RingConfig {
        max_bytes: 10_000,
        max_age: Duration::from_secs(60),
        max_records: Some(10),
    });
    let t0 = Instant::now();
    // Simulate a drain that already dropped mid-stream before ring push:
    // sequences 0 then 5.
    ring.push(rec(TOPIC_STDOUT, 0, b"0\n"), t0);
    ring.push(rec(TOPIC_STDOUT, 5, b"5\n"), t0);

    let mut sub = ring.subscribe(TopicFilter::all());
    assert!(matches!(sub.poll(&ring), Some(RingEvent::Record(_))));
    match sub.poll(&ring) {
        Some(RingEvent::Gap(g)) => {
            assert_eq!(g.reason, GapReason::OverflowDropOldest);
            assert_eq!(g.from_sequence, 1);
            assert_eq!(g.to_sequence, 5);
        }
        other => panic!("expected sequence gap, got {other:?}"),
    }
    assert!(matches!(
        sub.poll(&ring),
        Some(RingEvent::Record(r)) if r.stream_sequence == 5
    ));
}

#[test]
fn age_window_evicts_old_records() {
    let mut ring = LocalRing::new(RingConfig {
        max_bytes: 10_000,
        max_age: Duration::from_millis(50),
        max_records: None,
    });
    let t0 = Instant::now();
    ring.push(rec(TOPIC_STDOUT, 0, b"old\n"), t0);
    let t1 = t0 + Duration::from_millis(100);
    ring.push(rec(TOPIC_STDOUT, 1, b"new\n"), t1);
    assert_eq!(ring.len(), 1);
    let mut sub = ring.subscribe(TopicFilter::all());
    match sub.poll(&ring) {
        Some(RingEvent::Record(r)) => assert_eq!(r.bytes, b"new\n"),
        other => panic!("expected sole new record, got {other:?}"),
    }
    assert!(sub.poll(&ring).is_none());
}

#[test]
fn stream_drain_into_ring_under_byte_pressure() {
    let mut ring = LocalRing::new(RingConfig {
        max_bytes: 256,
        max_age: Duration::from_secs(60),
        max_records: None,
    });
    let mut drain = StreamDrain::new(StreamKind::Stdout).unwrap();
    for i in 0..40 {
        let line = format!("line-{i:02}\n");
        drain.push(line.as_bytes(), &mut ring);
    }
    drain.finish(&mut ring);
    assert!(ring.dropped_oldest() > 0);
    assert!(ring.total_bytes() <= 512);
    let mut sub = ring.subscribe(TopicFilter::all());
    let mut n = 0;
    while let Some(ev) = sub.poll(&ring) {
        if matches!(ev, RingEvent::Record(_)) {
            n += 1;
        }
    }
    assert!(n >= 1);
}
