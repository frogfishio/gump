//! GUMP-N017 / H01–H03 / H05–H06: one-node Hiccup discovery evidence.
//!
//! Maps INV-019–INV-024 (docs/v1/CONFORMANCE.md §3).

use std::sync::{Arc, Mutex};

use gump_hiccup::{
    AttemptSession, CanonicalTopic, Declaration, Detection, HealthInbound, HiccupToken,
    OutboundHealth, PlacementStamp, PresenceBoard, SdkConfig, SdkMiddleware, assert_self_isolation,
    detect_health_response, handle_successful_health, is_legacy_health, media_type,
    parse_declaration, plan_outbound_for, presence_ttl_ms,
};
use gump_types::{
    AttemptId, CapsuleId, Clock, ClusterId, DurationMillis, ExecutionId, InstantMillis,
    ManualClock, NodeId, UnitId, WorkloadId,
};

fn stamp(
    workload: WorkloadId,
    unit: UnitId,
    attempt: AttemptId,
    ip: &str,
    fence: u8,
) -> PlacementStamp {
    PlacementStamp {
        cluster_id: ClusterId::new(),
        namespace: "default".into(),
        app_id: "demo".into(),
        workload_id: workload,
        capsule_id: CapsuleId::new(),
        execution_id: ExecutionId::new(),
        unit_id: unit,
        role: None,
        rank: None,
        attempt_id: attempt,
        node_id: NodeId::new(),
        agent_incarnation: 1,
        placement_fence_digest: [fence; 32],
        health_eligible: true,
        receiver_reachable_ip: Some(ip.into()),
    }
}

fn allow_all_topics(_: &CanonicalTopic) -> bool {
    true
}

fn allow_all_publish(_: &gump_hiccup::ResolvedTopics) -> bool {
    true
}

fn apply(
    session: &mut AttemptSession,
    board: &mut PresenceBoard,
    s: PlacementStamp,
    body: &[u8],
    interval: u64,
    now: InstantMillis,
) -> gump_hiccup::InboundOutcome {
    handle_successful_health(
        session,
        board,
        HealthInbound {
            stamp: s,
            content_type: Some(media_type()),
            body,
            health_interval_ms: interval,
            now,
        },
        allow_all_publish,
        allow_all_topics,
    )
}

#[test]
fn inv019_legacy_health_unchanged_without_exact_declaration() {
    assert!(is_legacy_health(None));
    assert!(is_legacy_health(Some("text/plain")));
    assert!(is_legacy_health(Some("application/json")));
    assert!(!is_legacy_health(Some(media_type())));

    assert!(matches!(
        detect_health_response(Some("application/json"), br#"{"ok":true}"#),
        Detection::Inactive
    ));
    assert!(matches!(
        detect_health_response(Some(media_type()), br#"{"hiccup":1}"#),
        Detection::Active(_)
    ));
    assert!(matches!(
        detect_health_response(Some("application/json"), br#"{"hiccup":1}"#),
        Detection::Inactive
    ));
}

#[test]
fn inv020_sender_and_ip_only_from_placement_stamp() {
    let workload = WorkloadId::new();
    let unit = UnitId::new();
    let attempt = AttemptId::new();
    let mut board = PresenceBoard::new();
    let mut session = AttemptSession::new(workload);
    let s = stamp(workload, unit, attempt, "10.20.4.12", 1);
    let forged = br#"{"hiccup":1,"from":{"id":"00000000-0000-7000-8000-000000000001","attempt":"00000000-0000-7000-8000-000000000002","ip":"1.2.3.4"}}"#;
    let out = handle_successful_health(
        &mut session,
        &mut board,
        HealthInbound {
            stamp: s.clone(),
            content_type: Some(media_type()),
            body: forged,
            health_interval_ms: 10_000,
            now: InstantMillis::from_millis(0),
        },
        allow_all_publish,
        allow_all_topics,
    );
    assert!(out.degraded);
    assert_eq!(
        board.publisher_count(&CanonicalTopic::self_for(workload)),
        0
    );

    let out = apply(
        &mut session,
        &mut board,
        s,
        br#"{"hiccup":1}"#,
        10_000,
        InstantMillis::from_millis(0),
    );
    assert!(out.discovery_active);
    let peer = stamp(workload, UnitId::new(), AttemptId::new(), "10.20.4.99", 2);
    let mut peer_session = AttemptSession::new(workload);
    let _ = apply(
        &mut peer_session,
        &mut board,
        peer,
        br#"{"hiccup":1}"#,
        10_000,
        InstantMillis::from_millis(1),
    );
    let (d2, _) = board.deliver(&session.listen, attempt, workload, 0, allow_all_topics);
    let msg = d2
        .messages
        .iter()
        .find(|m| m.from.ip.as_deref() == Some("10.20.4.99"));
    assert!(msg.is_some(), "Gump-stamped peer IP must appear");
    assert_eq!(msg.unwrap().from.id.len(), 36);
}

#[test]
fn inv021_wrong_token_receives_no_discovery_view() {
    let token = HiccupToken::from_bytes([9u8; 32]);
    let wrong = HiccupToken::from_bytes([1u8; 32]);
    let mut sdk = SdkMiddleware::new(
        token,
        SdkConfig {
            declaration: Declaration::default(),
            dedupe: true,
        },
    );
    let body = br#"{"hiccup":1,"messages":[{"topic":"@self","from":{"id":"0198c6ef-5d5a-7d80-9ca0-54dc88879a35","attempt":"0198c6ef-5d5a-7d80-9ca0-54dc88879a36","ip":"10.0.0.1"}}],"more":false}"#;
    let seen = Arc::new(Mutex::new(0u32));
    let seen2 = seen.clone();
    let bad_auth = wrong.authorization_header_value();
    let resp = sdk.handle(
        "POST",
        &[("Authorization", bad_auth.as_str())],
        body,
        |_| *seen2.lock().unwrap() += 1,
    );
    assert_eq!(resp.status, 401);
    assert_eq!(*seen.lock().unwrap(), 0);

    let seen3 = seen.clone();
    let good = sdk.token().authorization_header_value();
    let resp = sdk.handle("POST", &[("Authorization", good.as_str())], body, |_| {
        *seen3.lock().unwrap() += 1
    });
    assert_eq!(resp.status, 200);
    assert_eq!(*seen.lock().unwrap(), 1);
}

#[test]
fn inv022_token_and_secret_never_in_debug_or_diagnostics() {
    let token = HiccupToken::from_bytes([0xab; 32]);
    let dbg = format!("{token:?}");
    assert!(!dbg.contains("ab"));
    assert_eq!(dbg, "HiccupToken(***)");
    let board = PresenceBoard::new();
    assert_eq!(board.omit_count, 0);
    let _ = board.approx_bytes();
}

#[test]
fn inv023_fenced_attempt_cannot_refresh_replacement_new_incarnation() {
    let workload = WorkloadId::new();
    let unit = UnitId::new();
    let attempt = AttemptId::new();
    let mut board = PresenceBoard::new();
    let mut session = AttemptSession::new(workload);
    let s = stamp(workload, unit, attempt, "10.0.0.1", 1);
    let _ = apply(
        &mut session,
        &mut board,
        s,
        br#"{"hiccup":1}"#,
        10_000,
        InstantMillis::from_millis(0),
    );
    assert_eq!(
        board.publisher_count(&CanonicalTopic::self_for(workload)),
        1
    );
    board.fence_attempt(attempt, &[1u8; 32]);
    assert_eq!(
        board.publisher_count(&CanonicalTopic::self_for(workload)),
        0
    );

    let attempt2 = AttemptId::new();
    let s2 = stamp(workload, unit, attempt2, "10.0.0.2", 2);
    let mut session2 = AttemptSession::new(workload);
    let _ = apply(
        &mut session2,
        &mut board,
        s2,
        br#"{"hiccup":1}"#,
        10_000,
        InstantMillis::from_millis(5),
    );
    assert_eq!(
        board.publisher_count(&CanonicalTopic::self_for(workload)),
        1
    );
}

#[test]
fn inv024_self_cannot_cross_workload_identity() {
    let a = WorkloadId::new();
    let b = WorkloadId::new();
    assert!(assert_self_isolation(a, b).is_ok());
    assert_ne!(
        CanonicalTopic::self_for(a).as_str(),
        CanonicalTopic::self_for(b).as_str()
    );

    let mut board = PresenceBoard::new();
    let mut sa = AttemptSession::new(a);
    let mut sb = AttemptSession::new(b);
    let _ = apply(
        &mut sa,
        &mut board,
        stamp(a, UnitId::new(), AttemptId::new(), "10.0.0.1", 1),
        br#"{"hiccup":1}"#,
        10_000,
        InstantMillis::from_millis(0),
    );
    let _ = apply(
        &mut sb,
        &mut board,
        stamp(b, UnitId::new(), AttemptId::new(), "10.0.0.2", 1),
        br#"{"hiccup":1}"#,
        10_000,
        InstantMillis::from_millis(1),
    );
    assert_eq!(board.publisher_count(&CanonicalTopic::self_for(a)), 1);
    assert_eq!(board.publisher_count(&CanonicalTopic::self_for(b)), 1);
    let (da, _) = board.deliver(&sa.listen, AttemptId::new(), a, 0, allow_all_topics);
    assert_eq!(sa.listen[0], CanonicalTopic::self_for(a));
    assert!(
        da.messages
            .iter()
            .all(|m| m.from.ip.as_deref() != Some("10.0.0.2"))
    );
}

#[test]
fn two_self_instances_receive_stamped_introductions() {
    let workload = WorkloadId::new();
    let mut board = PresenceBoard::new();
    let a_attempt = AttemptId::new();
    let b_attempt = AttemptId::new();
    let a_unit = UnitId::new();
    let b_unit = UnitId::new();
    let mut sa = AttemptSession::new(workload);
    let mut sb = AttemptSession::new(workload);
    let body = br#"{"hiccup":1}"#;
    let _ = apply(
        &mut sa,
        &mut board,
        stamp(workload, a_unit, a_attempt, "10.1.0.1", 1),
        body,
        5_000,
        InstantMillis::from_millis(0),
    );
    let out_b = apply(
        &mut sb,
        &mut board,
        stamp(workload, b_unit, b_attempt, "10.1.0.2", 1),
        body,
        5_000,
        InstantMillis::from_millis(1),
    );
    assert!(out_b.discovery_active);
    let (da, _) = board.deliver(&sa.listen, a_attempt, workload, 0, allow_all_topics);
    let (db, _) = board.deliver(&sb.listen, b_attempt, workload, 0, allow_all_topics);
    assert!(
        da.messages
            .iter()
            .any(|m| m.from.attempt == b_attempt.to_hyphenated())
    );
    assert!(
        db.messages
            .iter()
            .any(|m| m.from.attempt == a_attempt.to_hyphenated())
    );
    assert!(
        da.messages
            .iter()
            .any(|m| m.from.ip.as_deref() == Some("10.1.0.2"))
    );
}

#[test]
fn latest_presence_replaces_and_health_derived_expiry() {
    let clock = ManualClock::new(0);
    let workload = WorkloadId::new();
    let attempt = AttemptId::new();
    let mut board = PresenceBoard::new();
    let mut session = AttemptSession::new(workload);
    let interval = 10_000u64;
    let ttl = presence_ttl_ms(interval);
    assert_eq!(ttl, 30_000);

    let s = stamp(workload, UnitId::new(), attempt, "10.0.0.1", 1);
    let _ = apply(
        &mut session,
        &mut board,
        s.clone(),
        br#"{"hiccup":1,"data":{"v":1}}"#,
        interval,
        clock.now(),
    );
    let _ = apply(
        &mut session,
        &mut board,
        s,
        br#"{"hiccup":1,"data":{"v":2}}"#,
        interval,
        clock.now(),
    );
    assert_eq!(
        board.publisher_count(&CanonicalTopic::self_for(workload)),
        1
    );

    clock.advance(DurationMillis::from_millis(ttl + 1));
    board.expire(clock.now());
    assert_eq!(
        board.publisher_count(&CanonicalTopic::self_for(workload)),
        0
    );
}

#[test]
fn wrong_topic_policy_yields_no_view() {
    let workload = WorkloadId::new();
    let mut board = PresenceBoard::new();
    let mut session = AttemptSession::new(workload);
    let attempt = AttemptId::new();
    let _ = handle_successful_health(
        &mut session,
        &mut board,
        HealthInbound {
            stamp: stamp(workload, UnitId::new(), attempt, "10.0.0.1", 1),
            content_type: Some(media_type()),
            body: br#"{"hiccup":1,"topic":"banana","listen":["banana"]}"#,
            health_interval_ms: 10_000,
            now: InstantMillis::from_millis(0),
        },
        allow_all_publish,
        |_| false,
    );
    let (d, _) = board.deliver(&session.listen, attempt, workload, 0, |_| false);
    assert!(d.messages.is_empty());
}

#[test]
fn outbound_switches_to_authenticated_post_when_active() {
    let workload = WorkloadId::new();
    let attempt = AttemptId::new();
    let mut board = PresenceBoard::new();
    let mut session = AttemptSession::with_token(workload, HiccupToken::from_bytes([3u8; 32]));
    let before = plan_outbound_for(&session, attempt, &board, allow_all_topics);
    assert!(matches!(before, OutboundHealth::Get { offer: true }));
    let _ = apply(
        &mut session,
        &mut board,
        stamp(workload, UnitId::new(), attempt, "10.0.0.1", 1),
        br#"{"hiccup":1}"#,
        10_000,
        InstantMillis::from_millis(0),
    );
    let after = plan_outbound_for(&session, attempt, &board, allow_all_topics);
    match after {
        OutboundHealth::Post {
            authorization,
            content_type,
            ..
        } => {
            assert!(authorization.starts_with("Hiccup "));
            assert_eq!(content_type, media_type());
        }
        other => panic!("expected POST, got {other:?}"),
    }
}

#[test]
fn capability_mode_receives_complete_cross_workload_directory() {
    let mut board = PresenceBoard::new();
    let kismet_workload = WorkloadId::new();
    let ringtail_workload = WorkloadId::new();
    let consumer_workload = WorkloadId::new();
    let mut kismet = AttemptSession::new(kismet_workload);
    let mut ringtail = AttemptSession::new(ringtail_workload);
    let mut consumer = AttemptSession::new(consumer_workload);

    let _ = apply(
        &mut kismet,
        &mut board,
        stamp(
            kismet_workload,
            UnitId::new(),
            AttemptId::new(),
            "10.0.0.10",
            1,
        ),
        br#"{"hiccup":1,"capabilities":{"kismet.cluster/1":{"nodeId":"abc","port":7600},"kismet.ingress/1":{"port":443}}}"#,
        10_000,
        InstantMillis::from_millis(0),
    );
    let _ = apply(
        &mut ringtail,
        &mut board,
        stamp(
            ringtail_workload,
            UnitId::new(),
            AttemptId::new(),
            "10.0.0.20",
            1,
        ),
        br#"{"hiccup":1,"capabilities":{"ratatouille.sink/1":{"port":8081,"path":"/sink"}}}"#,
        10_000,
        InstantMillis::from_millis(1),
    );
    let out = apply(
        &mut consumer,
        &mut board,
        stamp(
            consumer_workload,
            UnitId::new(),
            AttemptId::new(),
            "10.0.0.30",
            1,
        ),
        br#"{"hiccup":1,"capabilities":{}}"#,
        10_000,
        InstantMillis::from_millis(2),
    );

    assert!(consumer.directory_mode);
    let directory = out.delivery.expect("initial directory");
    assert_eq!(directory.messages.len(), 3);
    let topics: std::collections::BTreeSet<_> = directory
        .messages
        .iter()
        .map(|message| message.topic.as_str())
        .collect();
    assert_eq!(
        topics,
        std::collections::BTreeSet::from([
            "kismet.cluster/1",
            "kismet.ingress/1",
            "ratatouille.sink/1",
        ])
    );
    assert!(
        directory
            .messages
            .iter()
            .all(|message| message.from.id.len() == 36 && message.from.attempt.len() == 36)
    );
    assert!(directory.messages.iter().any(|message| {
        message.topic == "ratatouille.sink/1"
            && message.from.ip.as_deref() == Some("10.0.0.20")
            && message
                .capabilities
                .get("ratatouille.sink/1")
                .and_then(|data| data.get("port"))
                == Some(&serde_json::json!(8081))
            && message.data.as_ref().and_then(|data| data.get("port"))
                == Some(&serde_json::json!(8081))
    }));
}

#[test]
fn http_origin_is_delivered_with_gump_stamps_and_new_capability_shape() {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Pilot5Delivery {
        hiccup: u8,
        messages: Vec<Pilot5Message>,
        more: bool,
    }

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Pilot5Message {
        topic: String,
        from: Pilot5Sender,
        #[serde(default)]
        capabilities: std::collections::BTreeMap<String, serde_json::Value>,
        #[serde(default)]
        data: Option<serde_json::Value>,
        #[serde(rename = "secretData")]
        secret_data: Option<String>,
    }

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Pilot5Sender {
        id: String,
        attempt: String,
        ip: Option<std::net::IpAddr>,
    }

    let mut board = PresenceBoard::new();
    let origin_workload = WorkloadId::new();
    let kismet_workload = WorkloadId::new();
    let origin_unit = UnitId::new();
    let origin_attempt = AttemptId::new();
    let mut origin = AttemptSession::new(origin_workload);
    let mut kismet = AttemptSession::new(kismet_workload);

    let _ = apply(
        &mut origin,
        &mut board,
        stamp(
            origin_workload,
            origin_unit,
            origin_attempt,
            "10.20.4.12",
            1,
        ),
        br#"{"hiccup":1,"capabilities":{"http.origin/1":{"port":8080,"domains":["abc.com","cde.org","def.net"]}}}"#,
        10_000,
        InstantMillis::from_millis(0),
    );
    let delivered = apply(
        &mut kismet,
        &mut board,
        stamp(
            kismet_workload,
            UnitId::new(),
            AttemptId::new(),
            "10.20.4.20",
            1,
        ),
        br#"{"hiccup":1,"capabilities":{"kismet.cluster/1":{"nodeId":"11111111111111111111111111111111","port":7600}}}"#,
        10_000,
        InstantMillis::from_millis(1),
    )
    .delivery
    .expect("directory delivery");

    let entry = delivered
        .messages
        .iter()
        .find(|message| message.topic == "http.origin/1")
        .expect("HTTP origin entry");
    assert_eq!(entry.from.id, origin_unit.to_string());
    assert_eq!(entry.from.attempt, origin_attempt.to_string());
    assert_eq!(entry.from.ip.as_deref(), Some("10.20.4.12"));
    assert_eq!(
        entry.capabilities.get("http.origin/1"),
        Some(&serde_json::json!({
            "port": 8080,
            "domains": ["abc.com", "cde.org", "def.net"]
        }))
    );

    // Mirror Pilot 5's deny-unknown-fields decoder so the checked wire bytes,
    // not only Gump's in-memory representation, form the handoff contract.
    let wire = gump_hiccup::encode_delivery(&delivered).expect("encode delivery");
    let pilot: Pilot5Delivery = serde_json::from_slice(&wire).expect("Pilot 5 wire shape");
    assert_eq!(pilot.hiccup, 1);
    assert!(!pilot.more);
    let pilot_entry = pilot
        .messages
        .iter()
        .find(|message| message.capabilities.contains_key("http.origin/1"))
        .expect("Pilot 5 HTTP origin");
    assert_eq!(pilot_entry.topic, "http.origin/1");
    assert_eq!(pilot_entry.from.id, origin_unit.to_string());
    assert_eq!(pilot_entry.from.attempt, origin_attempt.to_string());
    assert_eq!(
        pilot_entry.from.ip,
        Some("10.20.4.12".parse().expect("private IP"))
    );
    assert!(pilot_entry.data.is_some());
    assert!(pilot_entry.secret_data.is_none());
}

#[test]
fn legacy_topic_mode_remains_selective() {
    let workload = WorkloadId::new();
    let mut board = PresenceBoard::new();
    let capability_workload = WorkloadId::new();
    let mut capability_provider = AttemptSession::new(capability_workload);
    let _ = apply(
        &mut capability_provider,
        &mut board,
        stamp(
            capability_workload,
            UnitId::new(),
            AttemptId::new(),
            "10.0.0.40",
            1,
        ),
        br#"{"hiccup":1,"capabilities":{"ratatouille.sink/1":{"port":8081}}}"#,
        10_000,
        InstantMillis::from_millis(0),
    );
    let attempt = AttemptId::new();
    let mut legacy = AttemptSession::new(workload);
    let out = apply(
        &mut legacy,
        &mut board,
        stamp(workload, UnitId::new(), attempt, "10.0.0.50", 1),
        br#"{"hiccup":1}"#,
        10_000,
        InstantMillis::from_millis(1),
    );
    assert!(!legacy.directory_mode);
    assert!(out.delivery.expect("legacy delivery").messages.is_empty());
}

#[test]
fn corpus_examples_parse() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let decl = std::fs::read(root.join("spec/v1/hiccup/response.example.json")).unwrap();
    parse_declaration(&decl).unwrap();
}
