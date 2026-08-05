//! Telemetry keeper selection via rendezvous hashing (D011 / TELEMETRY.md §12).

use std::cmp::Ordering;

/// Cluster node identity used for keeper assignment.
pub type NodeId = u64;

/// Desired keeper replicas when the cluster is large enough (D011).
pub const TARGET_KEEPER_REPLICAS: usize = 2;

/// Minimum live nodes before rendezvous selects exactly [`TARGET_KEEPER_REPLICAS`].
/// Below this, every surviving node is eligible.
pub const RENDEZVOUS_MIN_NODES: usize = 3;

/// Select telemetry keepers for a logical shard key (app/attempt identity).
///
/// - `< 3` nodes → all surviving nodes (best-effort redundancy).
/// - `≥ 3` nodes → exactly two keepers by highest rendezvous score.
pub fn select_keepers(shard_key: &[u8], nodes: &[NodeId]) -> Vec<NodeId> {
    if nodes.is_empty() {
        return Vec::new();
    }
    let mut uniq = nodes.to_vec();
    uniq.sort_unstable();
    uniq.dedup();
    if uniq.len() < RENDEZVOUS_MIN_NODES {
        return uniq;
    }
    let mut scored: Vec<(NodeId, [u8; 32])> = uniq
        .iter()
        .copied()
        .map(|n| (n, rendezvous_score(shard_key, n)))
        .collect();
    scored.sort_by(|a, b| cmp_score_desc(a, b));
    scored
        .into_iter()
        .take(TARGET_KEEPER_REPLICAS)
        .map(|(n, _)| n)
        .collect()
}

fn rendezvous_score(shard_key: &[u8], node: NodeId) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(shard_key);
    hasher.update(&node.to_le_bytes());
    *hasher.finalize().as_bytes()
}

fn cmp_score_desc(a: &(NodeId, [u8; 32]), b: &(NodeId, [u8; 32])) -> Ordering {
    // Higher digest first; tie-break on node id for stability.
    match b.1.cmp(&a.1) {
        Ordering::Equal => a.0.cmp(&b.0),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_cluster_uses_all_nodes() {
        assert_eq!(select_keepers(b"app", &[1]), vec![1]);
        assert_eq!(select_keepers(b"app", &[2, 1]), vec![1, 2]);
    }

    #[test]
    fn three_plus_selects_exactly_two_stable() {
        let nodes = [10u64, 20, 30, 40];
        let a = select_keepers(b"shard-a", &nodes);
        let b = select_keepers(b"shard-a", &nodes);
        assert_eq!(a.len(), 2);
        assert_eq!(a, b);
        let other = select_keepers(b"shard-b", &nodes);
        assert_eq!(other.len(), 2);
    }
}
