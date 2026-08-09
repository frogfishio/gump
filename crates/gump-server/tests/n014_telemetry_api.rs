//! GUMP-N014: local `gump telemetry` recent-window poll over the Unix API.
//!
//! Authority: docs/v1/NEXT_ACTIONS.md GUMP-N014, TELEMETRY.md §12, INV-009.

use std::time::{Duration, Instant};

use gump_cli::{LocalRequest, LocalResponse, TelemetryEventBody};
use gump_server::{LocalDaemon, PeerAllowlist, handle_request};
use gump_telemetry::{
    ChunkFlags, RingConfig, StreamRecord, TOPIC_STDOUT, TelemetryPlane, TopicError,
};

#[test]
fn telemetry_disabled_fail_closed() {
    let daemon = LocalDaemon::new(PeerAllowlist::same_uid(1));
    let resp = handle_request(
        &daemon,
        LocalRequest::Telemetry {
            filter: None,
            max_events: None,
        },
    );
    assert!(matches!(
        resp,
        LocalResponse::Error(ref e) if e.reason == "telemetry.disabled"
    ));
}

#[test]
fn recent_window_filter_gaps_and_safe_identity() {
    let mut plane = TelemetryPlane::new(RingConfig {
        max_bytes: 10_000,
        max_age: Duration::from_secs(60),
        max_records: Some(2),
    });
    let t0 = Instant::now();
    for (seq, bytes) in [(0u64, b"one" as &[u8]), (1, b"two"), (2, b"three")] {
        plane.ring_mut().push(
            StreamRecord {
                topic: TOPIC_STDOUT,
                stream_sequence: seq,
                flags: ChunkFlags::BEGIN | ChunkFlags::END,
                bytes: bytes.to_vec(),
                utf8_hint: true,
                receive_offset: seq,
            },
            t0,
        );
    }
    assert!(plane.ring().dropped_oldest() >= 1);
    plane.emit_supervisor(b"attempt.started");

    // Forgery: application cannot emit reserved gump/ topics.
    assert!(matches!(
        plane.ingest_application_topic("gump/lifecycle", b"forged", 9),
        Err(TopicError::ReservedImpersonation)
    ));

    let daemon = LocalDaemon::new(PeerAllowlist::same_uid(1)).with_telemetry_plane(plane);
    let resp = handle_request(
        &daemon,
        LocalRequest::Telemetry {
            filter: Some("app/*".into()),
            max_events: Some(32),
        },
    );
    let LocalResponse::Telemetry {
        profile,
        memory_only,
        dropped_oldest,
        caught_up,
        identity_note,
        events,
        ..
    } = resp
    else {
        panic!("expected telemetry response, got {resp:?}");
    };
    assert_eq!(profile, "gump.ratatouille/1");
    assert!(memory_only);
    assert!(dropped_oldest >= 1);
    assert!(caught_up);
    assert!(identity_note.contains("placement-derived"));
    assert!(
        events.iter().any(
            |e| matches!(e, TelemetryEventBody::Record { topic, .. } if topic == "app/stdout")
        )
    );
    // Filtered out supervisor topic.
    assert!(events.iter().all(|e| match e {
        TelemetryEventBody::Record { topic, .. } | TelemetryEventBody::Gap { topic, .. } => {
            topic.starts_with("app/")
        }
    }));

    let gump = handle_request(
        &daemon,
        LocalRequest::Telemetry {
            filter: Some("gump/*".into()),
            max_events: Some(8),
        },
    );
    let LocalResponse::Telemetry { events, .. } = gump else {
        panic!("expected telemetry");
    };
    assert!(events.iter().any(
        |e| matches!(e, TelemetryEventBody::Record { topic, text: Some(t), .. }
                if topic == "gump/lifecycle" && t == "attempt.started")
    ));
}

#[test]
fn compose_enables_telemetry_plane_for_agent_roles() {
    use gump_server::{InitOptions, ProductRuntime};

    let rt = ProductRuntime::init(InitOptions {
        object_store: Some(gump_connectors::RuntimeObjectStore::Memory(
            gump_connectors::FakeObjectStore::new(),
        )),
        ..InitOptions::default()
    })
    .unwrap();
    assert!(rt.telemetry.enabled);
    assert!(rt.local_api.telemetry.is_some());
    let resp = handle_request(
        &rt.local_api,
        LocalRequest::Telemetry {
            filter: None,
            max_events: Some(1),
        },
    );
    assert!(matches!(
        resp,
        LocalResponse::Telemetry {
            memory_only: true,
            ..
        }
    ));
}
