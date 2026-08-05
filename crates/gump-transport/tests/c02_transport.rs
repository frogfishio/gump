//! C02 exit evidence: mTLS, limits, reconnect, certificate rotation.
//!
//! Authority: docs/v1/DELIVERY.md C02, DECISIONS D001/D007, PROTOCOL.md §2–§3.

use std::net::SocketAddr;
use std::time::Duration;

use gump_transport::{
    mint_identity, mint_identity_pair, prefer_session, NodeRole, OrderingPrefer, QuicEndpoint,
    ReconnectDecision, ReconnectPolicy, RotationAction, SessionSlot, TransportIdentity,
    TransportLimits,
};
use gump_types::{ClusterId, IncarnationId, NodeId};

fn fixed_v7(tag: u8) -> [u8; 16] {
    let mut b = [
        0x01, 0x8f, 0x4b, 0x00, 0x00, 0x00, 0x70, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ];
    b[15] = tag;
    b
}

fn identity(tag_node: u8, tag_inc: u8, roles: &[NodeRole]) -> TransportIdentity {
    TransportIdentity {
        cluster_id: ClusterId::from_bytes(fixed_v7(1)).unwrap(),
        node_id: NodeId::from_bytes(fixed_v7(tag_node)).unwrap(),
        incarnation: IncarnationId::from_bytes(fixed_v7(tag_inc)).unwrap(),
        roles: roles.to_vec(),
    }
}

#[test]
fn limits_reject_oversize_before_alloc() {
    let limits = TransportLimits::default();
    assert!(limits.check_control(1).is_ok());
    assert!(limits.check_control(limits.max_control_frame).is_ok());
    let err = limits
        .check_control(limits.max_control_frame + 1)
        .unwrap_err();
    assert!(matches!(
        err,
        gump_transport::TransportLimitError::ExceedsCeiling { .. }
    ));
    assert!(limits
        .check_bulk_chunk(limits.max_bulk_chunk + 1)
        .is_err());
    assert!(limits.check_hello(0).is_err());
}

#[test]
fn reconnect_backoff_then_give_up() {
    let policy = ReconnectPolicy {
        initial_delay: Duration::from_millis(10),
        max_delay: Duration::from_millis(40),
        multiplier_num: 2,
        multiplier_den: 1,
        max_attempts: 3,
    };
    match policy.after_failures(0) {
        ReconnectDecision::Retry { attempt, delay } => {
            assert_eq!(attempt, 0);
            assert_eq!(delay, Duration::from_millis(10));
        }
        other => panic!("unexpected {other:?}"),
    }
    match policy.after_failures(1) {
        ReconnectDecision::Retry { delay, .. } => {
            assert_eq!(delay, Duration::from_millis(20));
        }
        other => panic!("unexpected {other:?}"),
    }
    match policy.after_failures(2) {
        ReconnectDecision::Retry { delay, .. } => {
            assert_eq!(delay, Duration::from_millis(40));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(
        policy.after_failures(3),
        ReconnectDecision::GiveUp { attempts: 3 }
    );
}

#[test]
fn certificate_rotation_drains_previous_generation() {
    let a = identity(2, 3, &[NodeRole::Memory]);
    let slot = SessionSlot::active(a.clone(), 1);
    let rotated = TransportIdentity {
        incarnation: IncarnationId::from_bytes(fixed_v7(4)).unwrap(),
        ..a
    };
    let (plan, action) = slot.begin_rotation(rotated, 2).unwrap();
    assert_eq!(
        action,
        RotationAction::BeginDrain { from: 1, to: 2 }
    );
    assert!(matches!(plan.previous, SessionSlot::Draining { .. }));
    let (final_slot, done) = SessionSlot::complete_drain(&plan);
    assert_eq!(done, RotationAction::Complete { active: 2 });
    assert_eq!(final_slot.generation(), 2);

    let bad = slot
        .begin_rotation(
            TransportIdentity {
                cluster_id: ClusterId::from_bytes(fixed_v7(9)).unwrap(),
                ..identity(2, 5, &[NodeRole::Memory])
            },
            3,
        )
        .unwrap_err();
    assert_eq!(bad, RotationAction::Rejected("cluster mismatch"));
}

#[test]
fn duplicate_session_prefers_lexicographically_smaller_key() {
    let a = (NodeId::from_bytes(fixed_v7(1)).unwrap(), [1u8; 16]);
    let b = (NodeId::from_bytes(fixed_v7(2)).unwrap(), [0u8; 16]);
    assert_eq!(prefer_session(&a, &b), OrderingPrefer::KeepA);
    assert_eq!(prefer_session(&b, &a), OrderingPrefer::KeepB);
}

#[tokio::test]
async fn mtls_loopback_exchanges_control_frame() {
    let server_id = identity(10, 11, &[NodeRole::Memory, NodeRole::Controller]);
    let client_id = identity(12, 13, &[NodeRole::Agent]);
    let (server_mat, client_mat, ca) =
        mint_identity_pair(server_id.clone(), client_id.clone()).expect("mint pair");

    let limits = TransportLimits::default();
    let server = QuicEndpoint::server(
        &server_mat,
        &ca,
        "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        limits,
    )
    .unwrap();
    let addr = server.local_addr().unwrap();
    let client = QuicEndpoint::client(
        &client_mat,
        &ca,
        "127.0.0.1:0".parse().unwrap(),
        limits,
    )
    .unwrap();

    let server_task = tokio::spawn(async move {
        let sess = server.accept().await.unwrap();
        assert_eq!(sess.peer.node_id, client_id.node_id);
        assert_eq!(sess.peer.cluster_id, server_id.cluster_id);
        let (mut send, mut recv) = sess.accept_bi().await.unwrap();
        let body = sess.recv_control(&mut recv).await.unwrap();
        assert_eq!(body, b"hello-c02");
        sess.send_control(&mut send, b"ack").await.unwrap();
        sess
    });

    let sess = client.connect(addr).await.unwrap();
    assert_eq!(sess.peer.node_id, server_id.node_id);
    let (mut send, mut recv) = sess.open_bi().await.unwrap();
    sess.send_control(&mut send, b"hello-c02").await.unwrap();
    let ack = sess.recv_control(&mut recv).await.unwrap();
    assert_eq!(ack, b"ack");

    // Oversize rejected locally before write.
    let over = vec![0u8; limits.max_control_frame + 1];
    assert!(sess.send_control(&mut send, &over).await.is_err());

    let _server_sess = server_task.await.unwrap();
    sess.close();
}

#[tokio::test]
async fn mtls_rejects_untrusted_peer_ca() {
    let server_id = identity(20, 21, &[NodeRole::Memory]);
    let client_id = identity(22, 23, &[NodeRole::Agent]);
    let (server_mat, server_ca) = mint_identity(server_id).unwrap();
    let (client_mat, _client_ca) = mint_identity(client_id).unwrap();

    let limits = TransportLimits::default();
    let server = QuicEndpoint::server(
        &server_mat,
        &server_ca,
        "127.0.0.1:0".parse().unwrap(),
        limits,
    )
    .unwrap();
    let addr = server.local_addr().unwrap();
    // Client trusts its own CA, not the server's → connect must fail.
    let client = QuicEndpoint::client(
        &client_mat,
        &_client_ca,
        "127.0.0.1:0".parse().unwrap(),
        limits,
    )
    .unwrap();

    let accept = tokio::spawn(async move {
        let _ = server.accept().await;
    });
    match client.connect(addr).await {
        Ok(sess) => {
            accept.abort();
            panic!(
                "expected untrusted CA to fail handshake, got peer {:?}",
                sess.peer
            );
        }
        Err(err) => {
            let msg = err.to_string();
            assert!(
                !msg.is_empty(),
                "expected non-empty transport error"
            );
        }
    }
    accept.abort();
}

#[tokio::test]
async fn reconnect_after_close_establishes_new_session() {
    let server_id = identity(30, 31, &[NodeRole::Memory]);
    let client_id = identity(32, 33, &[NodeRole::Agent]);
    let (server_mat, client_mat, ca) = mint_identity_pair(server_id.clone(), client_id).unwrap();
    let limits = TransportLimits::default();
    let server = QuicEndpoint::server(
        &server_mat,
        &ca,
        "127.0.0.1:0".parse().unwrap(),
        limits,
    )
    .unwrap();
    let addr = server.local_addr().unwrap();
    let client = QuicEndpoint::client(
        &client_mat,
        &ca,
        "127.0.0.1:0".parse().unwrap(),
        limits,
    )
    .unwrap();

    let accept_loop = tokio::spawn(async move {
        for _ in 0..2 {
            let sess = server.accept().await.unwrap();
            sess.close();
        }
    });

    let policy = ReconnectPolicy::default();
    let s1 = client.connect(addr).await.unwrap();
    s1.close();
    assert!(matches!(
        policy.after_failures(0),
        ReconnectDecision::Retry { .. }
    ));
    let s2 = client.connect(addr).await.unwrap();
    assert_eq!(s2.peer.node_id, server_id.node_id);
    s2.close();
    accept_loop.await.unwrap();
}
