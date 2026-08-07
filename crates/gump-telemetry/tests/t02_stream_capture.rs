//! T02 exit evidence: long-line, binary, and saturation capture tests.
//!
//! Authority: docs/v1/DELIVERY.md T02, docs/v1/RUNTIME.md §14, D011.

use gump_telemetry::{
    BoundedRecordQueue, ChunkFlags, MAX_STREAM_RECORD_BYTES, StreamDrain, StreamKind, TOPIC_STDERR,
    TOPIC_STDOUT,
};

#[test]
fn topics_are_normative() {
    assert_eq!(StreamKind::Stdout.topic(), TOPIC_STDOUT);
    assert_eq!(StreamKind::Stderr.topic(), TOPIC_STDERR);
}

#[test]
fn short_lines_emit_as_single_begin_end_records() {
    let mut drain = StreamDrain::new(StreamKind::Stdout).unwrap();
    let mut q = BoundedRecordQueue::new(16);
    drain.push(b"hello\nworld\n", &mut q);
    drain.finish(&mut q);
    assert_eq!(q.records.len(), 2);
    assert_eq!(q.records[0].bytes, b"hello\n");
    assert_eq!(q.records[1].bytes, b"world\n");
    for r in &q.records {
        assert!(r.flags.contains(ChunkFlags::BEGIN));
        assert!(r.flags.contains(ChunkFlags::END));
        assert!(r.utf8_hint);
        assert_eq!(r.topic, TOPIC_STDOUT);
    }
}

#[test]
fn binary_bytes_including_nul_and_invalid_utf8_are_preserved() {
    let mut drain = StreamDrain::new(StreamKind::Stderr).unwrap();
    let mut q = BoundedRecordQueue::new(8);
    // NUL is valid UTF-8; 0xFF is not — both must round-trip unchanged.
    let blob = b"a\0b\xffc\n";
    drain.push(blob, &mut q);
    drain.finish(&mut q);
    assert_eq!(q.records.len(), 1);
    assert_eq!(q.records[0].bytes, blob);
    assert!(!q.records[0].utf8_hint);
    assert_eq!(q.records[0].topic, TOPIC_STDERR);
}

#[test]
fn long_line_is_chunked_without_utf8_requirement() {
    let mut drain = StreamDrain::new(StreamKind::Stdout).unwrap();
    let mut q = BoundedRecordQueue::new(64);
    let mut line = vec![b'x'; MAX_STREAM_RECORD_BYTES + 100];
    line.push(b'\n');
    // Push in 32 KiB pipe-sized pieces.
    let mut offset = 0;
    while offset < line.len() {
        let end = (offset + 32 * 1024).min(line.len());
        drain.push(&line[offset..end], &mut q);
        offset = end;
    }
    drain.finish(&mut q);
    assert!(q.records.len() >= 2, "expected chunked long line");
    let reconstructed: Vec<u8> = q
        .records
        .iter()
        .flat_map(|r| r.bytes.iter().copied())
        .collect();
    assert_eq!(reconstructed, line);
    assert!(q.records[0].flags.contains(ChunkFlags::BEGIN));
    assert!(q.records.back().unwrap().flags.contains(ChunkFlags::END));
    assert!(
        q.records
            .iter()
            .all(|r| r.bytes.len() <= MAX_STREAM_RECORD_BYTES)
    );
}

#[test]
fn saturation_drops_oldest_without_blocking() {
    let mut drain = StreamDrain::new(StreamKind::Stdout).unwrap();
    let mut q = BoundedRecordQueue::new(2);
    for i in 0..20 {
        let line = format!("line-{i}\n");
        drain.push(line.as_bytes(), &mut q);
    }
    drain.finish(&mut q);
    assert_eq!(q.records.len(), 2);
    assert!(q.dropped_oldest >= 18);
    // Drain never stalls: accepted counts every emit attempt.
    assert_eq!(q.accepted, 20);
    let texts: Vec<String> = q
        .records
        .iter()
        .map(|r| String::from_utf8(r.bytes.clone()).unwrap())
        .collect();
    assert_eq!(
        texts,
        vec!["line-18\n".to_string(), "line-19\n".to_string()]
    );
}

#[test]
fn stream_sequences_and_offsets_are_monotonic() {
    let mut drain = StreamDrain::new(StreamKind::Stdout).unwrap();
    let mut q = BoundedRecordQueue::new(8);
    drain.push(b"a\nb\n", &mut q);
    drain.finish(&mut q);
    assert_eq!(q.records[0].stream_sequence, 0);
    assert_eq!(q.records[1].stream_sequence, 1);
    assert_eq!(q.records[0].receive_offset, 0);
    assert_eq!(q.records[1].receive_offset, 2);
    assert_eq!(drain.receive_offset(), 4);
}
