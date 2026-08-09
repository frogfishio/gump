//! Live three-process-shape OpenRaft formation over QUIC mTLS.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use gump_memory::{ClusterJoinConfig, ClusterNetworkConfig, MemoryCluster, RaftCommand};
use gump_transport::{CaBundle, NodeRole, TransportIdentity, mint_identity_set};
use gump_types::Secret;
use gump_types::{ClusterId, IncarnationId, NodeId};

fn id_bytes(last: u8) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0] = 1;
    b[6] = 0x70;
    b[8] = 0x80;
    b[15] = last;
    b
}

fn reserved_addr() -> (UdpSocket, SocketAddr) {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let addr = socket.local_addr().unwrap();
    (socket, addr)
}

fn identity(cluster_id: ClusterId, node: u8) -> TransportIdentity {
    TransportIdentity {
        cluster_id,
        node_id: NodeId::from_bytes(id_bytes(node)).unwrap(),
        incarnation: IncarnationId::from_bytes(id_bytes(node + 10)).unwrap(),
        roles: vec![NodeRole::Memory, NodeRole::Agent],
    }
}

fn memory_id(node: u8) -> u64 {
    let id = NodeId::from_bytes(id_bytes(node)).unwrap();
    u64::from_be_bytes(id.as_bytes()[8..16].try_into().unwrap())
}

fn config(
    bind: SocketAddr,
    material: gump_transport::IdentityMaterial,
    ca: &CaBundle,
    join_tokens: BTreeMap<u64, Secret<String>>,
    join: Option<ClusterJoinConfig>,
) -> ClusterNetworkConfig {
    ClusterNetworkConfig {
        bind,
        advertise: bind,
        material,
        trust: ca.clone(),
        join_tokens,
        join,
    }
}

#[test]
fn three_nodes_join_promote_and_replicate_desired_state() {
    let cluster_id = ClusterId::from_bytes(id_bytes(42)).unwrap();
    let (mut materials, ca) = mint_identity_set(vec![
        identity(cluster_id, 1),
        identity(cluster_id, 2),
        identity(cluster_id, 3),
    ])
    .unwrap();
    let m3 = materials.pop().unwrap();
    let m2 = materials.pop().unwrap();
    let m1 = materials.pop().unwrap();
    let (s1, a1) = reserved_addr();
    let (s2, a2) = reserved_addr();
    let (s3, a3) = reserved_addr();
    drop(s1);
    let seed = MemoryCluster::bootstrap_networked(
        memory_id(1),
        1,
        config(
            a1,
            m1,
            &ca,
            BTreeMap::from([
                (memory_id(2), Secret::new("join-node-2".into())),
                (memory_id(3), Secret::new("join-node-3".into())),
            ]),
            None,
        ),
    )
    .unwrap();
    drop(s2);
    let node2 = MemoryCluster::bootstrap_networked(
        memory_id(2),
        2,
        config(
            a2,
            m2,
            &ca,
            BTreeMap::new(),
            Some(ClusterJoinConfig {
                seed: a1,
                token: Secret::new("join-node-2".into()),
            }),
        ),
    )
    .unwrap();
    drop(s3);
    let node3 = MemoryCluster::bootstrap_networked(
        memory_id(3),
        3,
        config(
            a3,
            m3,
            &ca,
            BTreeMap::new(),
            Some(ClusterJoinConfig {
                seed: a1,
                token: Secret::new("join-node-3".into()),
            }),
        ),
    )
    .unwrap();

    assert_eq!(seed.status_snapshot().unwrap().voter_count, 3);
    let response = seed
        .client_write(RaftCommand::PutDesired {
            namespace: "default".into(),
            app: "live-three".into(),
            expected_generation: 0,
            payload: b"binding".to_vec(),
            content_digest: [7; 32],
        })
        .unwrap();
    assert!(matches!(response, gump_memory::RaftResponse::Applied(_)));
    for _ in 0..50 {
        if node2.observed_desired_len() == 1 && node3.observed_desired_len() == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(node2.observed_desired_len(), 1);
    assert_eq!(node3.observed_desired_len(), 1);

    let unit_id = [11; 16];
    let completion = node2
        .client_write(RaftCommand::CompleteFinite {
            namespace: "default".into(),
            app: "live-three".into(),
            generation: 1,
            unit_id,
        })
        .unwrap();
    assert!(matches!(completion, gump_memory::RaftResponse::Applied(_)));
    for _ in 0..50 {
        if seed.observed_finite_completed("default", "live-three", 1, &unit_id)
            && node3.observed_finite_completed("default", "live-three", 1, &unit_id)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(seed.observed_finite_completed("default", "live-three", 1, &unit_id));
    assert!(node3.observed_finite_completed("default", "live-three", 1, &unit_id));

    node3.shutdown().unwrap();
    node2.shutdown().unwrap();
    seed.shutdown().unwrap();
}
