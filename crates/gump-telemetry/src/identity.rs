//! Authoritative Gump telemetry identity (never taken from app SourceIdentity).

use gump_types::{
    AttemptId, CapsuleId, ClusterId, ExecutionId, Label, NodeId, UnitId, WorkloadId,
};

/// Cluster telemetry profile name (docs/v1/README.md §4 / D011).
pub const TELEMETRY_PROFILE: &str = "gump.ratatouille/1";

/// Placement-derived identity attached by the agent before relay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalIdentity {
    pub cluster_id: ClusterId,
    pub namespace: Label,
    pub app_id: Label,
    pub workload_id: WorkloadId,
    pub release_id: CapsuleId,
    pub execution_id: ExecutionId,
    pub unit_id: UnitId,
    pub role: Option<Label>,
    pub rank: Option<u32>,
    pub attempt_id: AttemptId,
    pub node_id: NodeId,
    pub agent_incarnation: u64,
}

/// Application-supplied Ratatouille `SourceIdentity` retained under `producer`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProducerHint {
    pub app: Option<String>,
    pub r#where: Option<String>,
    pub instance: Option<String>,
}

impl ProducerHint {
    pub fn from_ratatouille(source: &ratatouille::SourceIdentity) -> Self {
        Self {
            app: source.app.clone(),
            r#where: source.r#where.clone(),
            instance: source.instance.clone(),
        }
    }
}

/// Normalized record ready for local relay / later TelemetryBatchV1.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizedRecord {
    pub profile: &'static str,
    pub topic: String,
    pub topic_sequence: Option<u64>,
    pub message: String,
    pub identity: CanonicalIdentity,
    pub producer: ProducerHint,
    pub local_sequence: u64,
}
