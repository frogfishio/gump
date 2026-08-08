//! T01 exit evidence: Ratatouille callback corpus + source-forgery tests.
//!
//! Authority: docs/v1/DELIVERY.md T01, DECISIONS D011, docs/v1/RUNTIME.md §14,
//! docs/TELEMETRY.md §3.

use gump_telemetry::{
    CallbackAdapter, CanonicalIdentity, MAX_RECORD_BYTES, ProducerHint, RecordOutcome,
    TELEMETRY_PROFILE, validate_topic,
};
use gump_types::{AttemptId, CapsuleId, ClusterId, ExecutionId, Label, NodeId, UnitId, WorkloadId};
use ratatouille::{EmitResult, Format, Logger, LoggerConfig, SourceIdentity};

/// Deterministic UUIDv7-shaped bytes for fixtures.
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
fn upstream_ndjson_callback_corpus() {
    let shared = CallbackAdapter::new(sample_identity()).into_shared();
    let config = LoggerConfig {
        filter: Some("app*,gump/*".into()),
        format: Format::Ndjson,
        source: SourceIdentity {
            app: Some("accounts".into()),
            r#where: Some("local".into()),
            instance: Some("run-1".into()),
        },
        ..Default::default()
    };

    let mut logger = Logger::with_sink(config, shared.as_fn_sink());
    assert_eq!(logger.log("app/stdout", "hello"), EmitResult::Emitted);
    assert_eq!(logger.log("noise", "nope"), EmitResult::Filtered);

    let records = shared.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].topic, "app/stdout");
    assert_eq!(records[0].message, "hello");
    assert_eq!(records[0].profile, TELEMETRY_PROFILE);
    assert_eq!(records[0].topic_sequence, Some(1));
    assert_eq!(records[0].producer.app.as_deref(), Some("accounts"));
    assert_eq!(records[0].identity.app_id.as_str(), "accounts");
    assert_eq!(shared.outcomes(), vec![RecordOutcome::Accepted]);
}

#[test]
fn source_forgery_cannot_replace_canonical_identity() {
    let shared = CallbackAdapter::new(sample_identity()).into_shared();
    let config = LoggerConfig {
        filter: Some("*".into()),
        format: Format::Ndjson,
        source: SourceIdentity {
            app: Some("evil-other-app".into()),
            r#where: Some("forged-where".into()),
            instance: Some("forged-instance".into()),
        },
        ..Default::default()
    };

    let mut logger = Logger::with_sink(config, shared.as_fn_sink());
    assert_eq!(logger.log("app/event", "pwn"), EmitResult::Emitted);

    let rec = &shared.records()[0];
    // Authoritative fields unchanged.
    assert_eq!(rec.identity.app_id.as_str(), "accounts");
    assert_eq!(rec.identity.namespace.as_str(), "default");
    assert_eq!(rec.identity.agent_incarnation, 9);
    assert_eq!(rec.identity.cluster_id.as_bytes(), &v7(1));
    // Forged source retained only as producer hint.
    assert_eq!(
        rec.producer,
        ProducerHint {
            app: Some("evil-other-app".into()),
            r#where: Some("forged-where".into()),
            instance: Some("forged-instance".into()),
        }
    );
}

#[test]
fn oversize_and_invalid_topics_rejected() {
    let mut adapter = CallbackAdapter::new(sample_identity());
    let huge = format!(
        "{{\"topic\":\"app/x\",\"args\":[\"{}\"]}}",
        "x".repeat(MAX_RECORD_BYTES)
    );
    assert_eq!(adapter.ingest_line(&huge), RecordOutcome::RejectedOversize);

    assert_eq!(
        adapter.ingest_line("{\"topic\":\"App/Bad\",\"args\":[\"x\"]}"),
        RecordOutcome::RejectedTopic
    );
    assert!(validate_topic("gump/lifecycle").is_ok());
}

#[test]
fn text_format_callback_path() {
    let mut adapter = CallbackAdapter::new(sample_identity());
    assert_eq!(
        adapter.ingest_line("app/stderr boom"),
        RecordOutcome::Accepted
    );
    assert_eq!(adapter.records()[0].topic, "app/stderr");
    assert_eq!(adapter.records()[0].message, "boom");
    assert!(adapter.records()[0].producer.app.is_none());
}
