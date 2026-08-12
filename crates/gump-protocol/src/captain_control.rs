//! Frozen first slice of the Captain–Gump runtime-control wire contract.

use serde::{Deserialize, Serialize};

pub const CAPTAIN_CONTROL_PROTOCOL: &str = "gump.captain-control/1";
pub const CAPTAIN_SNAPSHOT_SCHEMA: &str = "gump.captain-snapshot/1";
pub const CAPTAIN_ERROR_SCHEMA: &str = "gump.captain-error/1";
pub const MAX_SNAPSHOT_WORKLOADS: usize = 256;
pub const MAX_SNAPSHOT_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptainSnapshotV1 {
    pub schema: String,
    pub protocol: String,
    pub cluster_identity: String,
    pub node_identity: String,
    pub consistency: String,
    pub revision: u64,
    pub cluster: CaptainClusterSnapshotV1,
    pub workloads: Vec<CaptainWorkloadSnapshotV1>,
    pub local_execution: Option<CaptainLocalExecutionSnapshotV1>,
    pub limits: CaptainSnapshotLimitsV1,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptainClusterSnapshotV1 {
    pub raft_node_id: u64,
    pub current_leader: Option<u64>,
    pub voters: Vec<u64>,
    pub voter_count: u32,
    pub controller_epoch: u64,
    pub controller_holder: Option<u64>,
    pub durable_cluster_state: bool,
    pub custody: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptainWorkloadSnapshotV1 {
    pub namespace: String,
    pub app: String,
    pub generation: u64,
    pub capsule_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptainLocalExecutionSnapshotV1 {
    pub scope: String,
    pub desired: u64,
    pub placements: u64,
    pub completed: u64,
    pub ready: u64,
    pub hiccup_presence: u64,
    pub degraded: bool,
    pub s3_head_requests: u64,
    pub s3_full_get_requests: u64,
    pub s3_ranged_get_requests: u64,
    pub s3_bytes_read: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptainSnapshotLimitsV1 {
    pub max_workloads: u32,
    pub max_response_bytes: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptainControlErrorV1 {
    pub schema: String,
    pub code: String,
    pub retryable: bool,
    pub detail: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_wire_shape_is_camel_case_and_bounded() {
        let snapshot = CaptainSnapshotV1 {
            schema: CAPTAIN_SNAPSHOT_SCHEMA.into(),
            protocol: CAPTAIN_CONTROL_PROTOCOL.into(),
            cluster_identity: "cluster".into(),
            node_identity: "node".into(),
            consistency: "linearizable".into(),
            revision: 7,
            cluster: CaptainClusterSnapshotV1 {
                raft_node_id: 1,
                current_leader: Some(1),
                voters: vec![1],
                voter_count: 1,
                controller_epoch: 2,
                controller_holder: Some(1),
                durable_cluster_state: false,
                custody: "unsealed".into(),
            },
            workloads: Vec::new(),
            local_execution: None,
            limits: CaptainSnapshotLimitsV1 {
                max_workloads: MAX_SNAPSHOT_WORKLOADS as u32,
                max_response_bytes: MAX_SNAPSHOT_BYTES as u32,
            },
        };
        let bytes = serde_json::to_vec(&snapshot).unwrap();
        assert!(bytes.len() < MAX_SNAPSHOT_BYTES);
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\"clusterIdentity\""));
        assert!(!text.contains("cluster_identity"));
    }
}
