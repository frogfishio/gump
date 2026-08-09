//! STL-23: PutDesired budgets + idempotency index restore + oldest-by-time eviction.
//!
//! Authority: D014 / PROTOCOL §7 / STL-15 residual.

use gump_memory::{
    ClusterState, Command, DESIRED_MAX_PAYLOAD_BYTES, IDEMPOTENCY_TTL_MS, RaftCommand, RaftResponse,
};

fn op_id(bytes: [u8; 4]) -> [u8; 16] {
    let mut id = [0u8; 16];
    id[..4].copy_from_slice(&bytes);
    id
}

fn put_desired(ns: &str, app: &str, expected_generation: u64, payload: &[u8]) -> RaftCommand {
    RaftCommand::PutDesired {
        namespace: ns.into(),
        app: app.into(),
        expected_generation,
        payload: payload.to_vec(),
        content_digest: *blake3::hash(payload).as_bytes(),
    }
}

fn idem_with_id(id: [u8; 16], now_ms: u64) -> RaftCommand {
    RaftCommand::Idempotent {
        operation_id: id,
        request_digest: [9u8; 32],
        inner: Box::new(RaftCommand::Record(Command::AdvanceTime { now_ms })),
    }
}

#[test]
fn put_desired_rejects_oversized_payload() {
    let mut state = ClusterState::new();
    let huge = vec![0u8; DESIRED_MAX_PAYLOAD_BYTES + 1];
    let resp = state.apply(put_desired("default", "accounts", 0, &huge));
    assert!(
        matches!(resp, RaftResponse::Rejected(ref m) if m.contains("payload")),
        "got {resp:?}"
    );
    assert_eq!(state.desired_len(), 0);
}

#[test]
fn put_desired_rejects_invalid_labels_and_map_budget() {
    let mut state = ClusterState::with_desired_limits(1, 10_000);
    let bad = state.apply(put_desired("Bad_NS", "app", 0, b"x"));
    assert!(matches!(bad, RaftResponse::Rejected(_)));

    assert!(matches!(
        state.apply(put_desired("default", "app-a", 0, b"one")),
        RaftResponse::Applied(_)
    ));
    let full = state.apply(put_desired("default", "app-b", 0, b"two"));
    assert!(
        matches!(full, RaftResponse::Rejected(ref m) if m.contains("map full")),
        "got {full:?}"
    );

    let mut tight = ClusterState::with_desired_limits(8, 8);
    let over_bytes = tight.apply(put_desired("default", "app", 0, b"0123456789"));
    assert!(
        matches!(over_bytes, RaftResponse::Rejected(ref m) if m.contains("byte budget")),
        "got {over_bytes:?}"
    );
}

#[test]
fn finite_completion_is_generation_fenced_and_replacement_clears_it() {
    let mut state = ClusterState::new();
    assert!(matches!(
        state.apply(put_desired("default", "job", 0, b"one")),
        RaftResponse::Applied(_)
    ));
    let unit_id = [4; 16];
    assert!(matches!(
        state.apply(RaftCommand::CompleteFinite {
            namespace: "default".into(),
            app: "job".into(),
            generation: 1,
            unit_id,
        }),
        RaftResponse::Applied(_)
    ));
    assert!(state.finite_completed("default", "job", 1, &unit_id));
    assert!(matches!(
        state.apply(put_desired("default", "job", 1, b"two")),
        RaftResponse::Applied(_)
    ));
    assert!(!state.finite_completed("default", "job", 1, &unit_id));
    let stale = state.apply(RaftCommand::CompleteFinite {
        namespace: "default".into(),
        app: "job".into(),
        generation: 1,
        unit_id,
    });
    assert!(matches!(stale, RaftResponse::Rejected(_)));
}

#[test]
fn capacity_evicts_oldest_by_time_not_lex_min_id() {
    let mut state = ClusterState::with_idempotency_limits(2, IDEMPOTENCY_TTL_MS);
    let high_lex = op_id([0xff, 0xff, 0xff, 0xff]);
    let low_lex = op_id([0x00, 0x00, 0x00, 0x01]);

    // Older-by-time receipt uses the lexicographically *larger* id.
    assert!(matches!(
        state.apply(idem_with_id(high_lex, 10)),
        RaftResponse::Applied(_)
    ));
    assert!(matches!(
        state.apply(idem_with_id(low_lex, 20)),
        RaftResponse::Applied(_)
    ));
    // Third insert forces capacity eviction — must drop high_lex (t=10), keep low_lex.
    let third = op_id([0x00, 0x00, 0x00, 0x02]);
    assert!(matches!(
        state.apply(idem_with_id(third, 30)),
        RaftResponse::Applied(_)
    ));
    assert_eq!(state.idempotency_len(), 2);

    // Replay checks must not insert first — verify keep, then confirm evicted.
    let replay_keep = state.apply(idem_with_id(low_lex, 30));
    assert!(
        matches!(replay_keep, RaftResponse::Replay(_)),
        "newer low-lex id must remain; got {replay_keep:?}"
    );
    let replay_old = state.apply(idem_with_id(high_lex, 30));
    assert!(
        matches!(replay_old, RaftResponse::Applied(_)),
        "time-oldest (high lex) must have been evicted; got {replay_old:?}"
    );
}
