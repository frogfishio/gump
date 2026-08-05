//! C07 exit evidence: stale-effect replay suite (INV-007).
//!
//! Authority: docs/v1/DELIVERY.md C07, PROTOCOL.md §9, CONFORMANCE INV-007.

use gump_memory::{
    AgentFenceError, AgentFenceMemory, ControllerAuthority, ControllerError, EffectCommand,
    EffectReject, FenceToken, LeasePurpose, LeaseTable,
};

fn effect(token: FenceToken, op_id: u64) -> EffectCommand {
    EffectCommand {
        token,
        declaration_generation: 1,
        object_revision: 1,
        op_id,
    }
}

#[test]
fn acquire_controller_bumps_epoch_and_binds_lease() {
    let mut auth = ControllerAuthority::new();
    let mut leases = LeaseTable::default();
    let t0 = auth.acquire(1, 0, &mut leases);
    assert_eq!(t0.epoch, 1);
    assert_eq!(auth.holder(), Some(1));
    let lease = leases.get(t0.lease_id).unwrap();
    assert_eq!(lease.purpose, LeasePurpose::ControllerAuthority);
    assert_eq!(lease.ttl_ms, 10_000);

    let t1 = auth.acquire(2, 100, &mut leases);
    assert_eq!(t1.epoch, 2);
    assert_ne!(t0.fence, t1.fence);
    assert_eq!(auth.current().unwrap().epoch, 2);
}

#[test]
fn current_fence_accepts_matching_effect() {
    let mut auth = ControllerAuthority::new();
    let mut leases = LeaseTable::default();
    let token = auth.acquire(1, 0, &mut leases);
    let op = auth
        .accept_effect(&effect(token, 42), 0, &leases)
        .unwrap();
    assert_eq!(op, 42);
}

#[test]
fn stale_epoch_after_leader_change_rejected() {
    let mut auth = ControllerAuthority::new();
    let mut leases = LeaseTable::default();
    let old = auth.acquire(1, 0, &mut leases);
    let _new = auth.acquire(2, 50, &mut leases);

    let err = auth
        .accept_effect(&effect(old, 7), 50, &leases)
        .unwrap_err();
    assert!(matches!(
        err,
        ControllerError::Reject(EffectReject::StaleEpoch {
            current: 2,
            presented: 1
        })
    ));
}

#[test]
fn equal_epoch_different_fence_is_protocol_violation_on_cluster() {
    let mut auth = ControllerAuthority::new();
    let mut leases = LeaseTable::default();
    let token = auth.acquire(1, 0, &mut leases);
    let mut forged = token;
    forged.fence = [0xFF; 16];

    let err = auth
        .accept_effect(&effect(forged, 1), 0, &leases)
        .unwrap_err();
    assert!(matches!(
        err,
        ControllerError::Reject(EffectReject::FenceMismatch { epoch: 1 })
    ));
}

#[test]
fn expired_lease_cannot_create_effect() {
    let mut auth = ControllerAuthority::new();
    let mut leases = LeaseTable::default();
    let token = auth.acquire(1, 0, &mut leases);
    // Controller TTL is 10s; expire and revoke via ExpireLeases path on table.
    leases.expire_due(10_001);
    let err = auth
        .accept_effect(&effect(token, 1), 10_001, &leases)
        .unwrap_err();
    assert!(matches!(
        err,
        ControllerError::Reject(EffectReject::ExpiredOrUnverifiable { .. })
    ));
}

#[test]
fn agent_higher_epoch_permanently_fences_lower() {
    let mut agent = AgentFenceMemory::new();
    let mut auth = ControllerAuthority::new();
    let mut leases = LeaseTable::default();

    let e1 = auth.acquire(1, 0, &mut leases);
    agent.accept(e1).unwrap();
    agent.authorize_effect(&e1).unwrap();

    let e2 = auth.acquire(2, 10, &mut leases);
    agent.accept(e2).unwrap();

    // Stale replay of epoch-1 effect after leader change.
    assert!(matches!(
        agent.authorize_effect(&e1),
        Err(AgentFenceError::Reject(EffectReject::StaleEpoch {
            current: 2,
            presented: 1
        }))
    ));
    // Re-accepting lower epoch is refused for process lifetime.
    assert!(matches!(
        agent.accept(e1),
        Err(AgentFenceError::Reject(EffectReject::StaleEpoch { .. }))
    ));
}

#[test]
fn agent_equal_epoch_conflicting_fence_is_protocol_violation() {
    let mut agent = AgentFenceMemory::new();
    let token = FenceToken::new(3, FenceToken::derive_fence(3, 1, 1), 1);
    agent.accept(token).unwrap();
    let conflict = FenceToken::new(3, [9u8; 16], 1);
    assert!(matches!(
        agent.accept(conflict),
        Err(AgentFenceError::ProtocolViolation { epoch: 3 })
    ));
}

#[test]
fn stale_effect_replay_suite_across_leader_change() {
    // INV-007: delay and replay every effect command across leader change.
    let mut auth = ControllerAuthority::new();
    let mut leases = LeaseTable::default();
    let mut agent = AgentFenceMemory::new();

    let leader_a = auth.acquire(10, 0, &mut leases);
    agent.accept(leader_a).unwrap();

    let delayed_ops = [
        effect(leader_a, 100),
        effect(leader_a, 101),
        effect(leader_a, 102),
    ];
    // All accepted under current fence.
    for cmd in &delayed_ops {
        auth.accept_effect(cmd, 0, &leases).unwrap();
        agent.authorize_effect(&cmd.token).unwrap();
    }

    // Leader loss → new AcquireController.
    let leader_b = auth.acquire(20, 1_000, &mut leases);
    agent.accept(leader_b).unwrap();

    // Delayed replays under old fence create no accepted effect.
    for cmd in &delayed_ops {
        assert!(
            auth.accept_effect(cmd, 1_000, &leases).is_err(),
            "cluster must reject stale op {}",
            cmd.op_id
        );
        assert!(
            agent.authorize_effect(&cmd.token).is_err(),
            "agent must reject stale op {}",
            cmd.op_id
        );
    }

    // Fresh effects under new fence succeed.
    let fresh = effect(leader_b, 200);
    assert_eq!(auth.accept_effect(&fresh, 1_000, &leases).unwrap(), 200);
    agent.authorize_effect(&fresh.token).unwrap();
}
