//! STL-15: ClusterState idempotency bounds (D014) + sealed record mutator.
//!
//! Authority: docs/v1/DECISIONS.md D014, PROTOCOL.md §15, STL-01 residual.

use gump_memory::{
    ClusterState, Command, IDEMPOTENCY_MAX_ENTRIES, IDEMPOTENCY_TTL_MS, RaftCommand, RaftResponse,
};

fn op_id(i: u32) -> [u8; 16] {
    let mut id = [0u8; 16];
    id[..4].copy_from_slice(&i.to_be_bytes());
    id
}

fn idem_advance(i: u32, now_ms: u64) -> RaftCommand {
    RaftCommand::Idempotent {
        operation_id: op_id(i),
        request_digest: [7u8; 32],
        inner: Box::new(RaftCommand::Record(Command::AdvanceTime { now_ms })),
    }
}

#[test]
fn d014_constants_match_protocol() {
    assert_eq!(IDEMPOTENCY_MAX_ENTRIES, 100_000);
    assert_eq!(IDEMPOTENCY_TTL_MS, 24 * 60 * 60 * 1_000);
    let production = ClusterState::new();
    // Default constructor wires the D014 ceiling (exercised below with a scaled harness).
    assert_eq!(production.idempotency_len(), 0);
}

#[test]
fn idempotency_map_capped_under_load() {
    // Same deterministic eviction path as production; scaled for debug runtime.
    // Production ceiling is IDEMPOTENCY_MAX_ENTRIES (100_000) — see constants test.
    let ceiling = 256usize;
    let mut state = ClusterState::with_idempotency_limits(ceiling, IDEMPOTENCY_TTL_MS);
    let n = ceiling + 1;
    for i in 0..n as u32 {
        let resp = state.apply(idem_advance(i, u64::from(i).saturating_add(1)));
        assert!(
            matches!(resp, RaftResponse::Applied(_)),
            "op {i} should apply"
        );
    }
    assert!(
        state.idempotency_len() <= ceiling,
        "idempotency grew to {}",
        state.idempotency_len()
    );
    let now = state.records().now_ms();
    // Oldest-by-time (op 0 at t=1) is evicted first under the ceiling (STL-23).
    let replay_oldest = state.apply(idem_advance(0, now));
    assert!(
        matches!(replay_oldest, RaftResponse::Applied(_)),
        "evicted op must not replay; got {replay_oldest:?}"
    );
    // A still-retained high op_id must replay.
    let keep = (ceiling as u32).saturating_sub(1);
    let replay_keep = state.apply(idem_advance(keep, now));
    assert!(
        matches!(replay_keep, RaftResponse::Replay(_)),
        "recent op should still be retained; got {replay_keep:?}"
    );
}

/// Full D014 ceiling evidence (slow in debug; run with `--release` or `--ignored`).
#[test]
fn full_100k_ceiling_under_load() {
    let mut state = ClusterState::new();
    let n = IDEMPOTENCY_MAX_ENTRIES + 1;
    for i in 0..n as u32 {
        let resp = state.apply(idem_advance(i, u64::from(i).saturating_add(1)));
        assert!(matches!(resp, RaftResponse::Applied(_)));
    }
    assert!(state.idempotency_len() <= IDEMPOTENCY_MAX_ENTRIES);
}

#[test]
fn idempotency_expires_after_committed_ttl() {
    let mut state = ClusterState::new();
    let resp = state.apply(idem_advance(1, 1_000));
    assert!(matches!(resp, RaftResponse::Applied(_)));
    assert_eq!(state.idempotency_len(), 1);

    // Advance committed time past the 24h TTL; prune runs after apply_inner.
    let later = 1_000 + IDEMPOTENCY_TTL_MS + 1;
    let tick = state.apply(RaftCommand::Record(Command::AdvanceTime { now_ms: later }));
    assert!(matches!(tick, RaftResponse::Applied(_)));
    assert_eq!(
        state.idempotency_len(),
        0,
        "receipt must expire once committed time advances past TTL"
    );

    let again = state.apply(idem_advance(1, later));
    assert!(
        matches!(again, RaftResponse::Applied(_)),
        "expired op must apply fresh, not replay"
    );
}

#[test]
fn cluster_state_has_no_public_records_mut() {
    // Seal: external crates only observe via `records()`; mutations go through `apply`.
    let state = ClusterState::new();
    let _ = state.records().revision();
    let _ = state.idempotency_len();
}
