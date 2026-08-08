//! Gump-stamped identity / IP (HICCUP.md §4). Applications never supply `from`.

use gump_types::{AttemptId, CapsuleId, ClusterId, ExecutionId, NodeId, UnitId, WorkloadId};

use crate::codec::PublicFrom;
use crate::topic::CanonicalTopic;

/// Placement fields Gump trusts when stamping introductions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlacementStamp {
    pub cluster_id: ClusterId,
    pub namespace: String,
    pub app_id: String,
    pub workload_id: WorkloadId,
    pub capsule_id: CapsuleId,
    pub execution_id: ExecutionId,
    pub unit_id: UnitId,
    pub role: Option<String>,
    pub rank: Option<u32>,
    pub attempt_id: AttemptId,
    pub node_id: NodeId,
    pub agent_incarnation: u64,
    /// Opaque fence digest bytes (not application-visible in minimal JSON).
    pub placement_fence_digest: [u8; 32],
    pub health_eligible: bool,
    /// Node-private address selected for the *receiver* (may be absent).
    pub receiver_reachable_ip: Option<String>,
}

impl PlacementStamp {
    pub fn public_from(&self) -> PublicFrom {
        PublicFrom {
            id: self.unit_id.to_hyphenated(),
            attempt: self.attempt_id.to_hyphenated(),
            ip: self.receiver_reachable_ip.clone(),
        }
    }

    pub fn fence_matches(&self, digest: &[u8; 32]) -> bool {
        &self.placement_fence_digest == digest
    }
}

/// Display topic for applications: `@self` when canonical is workload-self, else named.
pub fn application_topic(canonical: &CanonicalTopic, workload: WorkloadId) -> String {
    let self_topic = CanonicalTopic::self_for(workload);
    if canonical == &self_topic {
        "@self".into()
    } else {
        canonical.as_str().to_string()
    }
}
