//! One-node Hiccup wiring on the agent reconcile path (GUMP-N017 / D016).

use std::collections::{BTreeMap, BTreeSet};

use gump_hiccup::{
    AttemptSession, HealthInbound, OutboundHealth, PlacementStamp, PresenceBoard,
    handle_successful_health, plan_outbound_for,
};
use gump_types::{
    AttemptId, CapsuleId, ClusterId, ExecutionId, InstantMillis, NodeId, UnitId, WorkloadId,
};

/// Optional placement fields required to stamp Hiccup introductions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HiccupPlacement {
    pub cluster_id: ClusterId,
    pub namespace: String,
    pub app_id: String,
    pub workload_id: WorkloadId,
    pub capsule_id: CapsuleId,
    pub execution_id: ExecutionId,
    pub node_id: NodeId,
    pub agent_incarnation: u64,
    pub private_ip: Option<String>,
}

impl HiccupPlacement {
    pub fn stamp(&self, unit_id: UnitId, attempt_id: AttemptId, fence: u64) -> PlacementStamp {
        let mut digest = [0u8; 32];
        digest[..8].copy_from_slice(&fence.to_le_bytes());
        PlacementStamp {
            cluster_id: self.cluster_id,
            namespace: self.namespace.clone(),
            app_id: self.app_id.clone(),
            workload_id: self.workload_id,
            capsule_id: self.capsule_id,
            execution_id: self.execution_id,
            unit_id,
            role: None,
            rank: None,
            attempt_id,
            node_id: self.node_id,
            agent_incarnation: self.agent_incarnation,
            placement_fence_digest: digest,
            health_eligible: true,
            receiver_reachable_ip: self.private_ip.clone(),
        }
    }
}

/// Agent-local one-node board + per-attempt sessions.
#[derive(Default)]
pub struct HiccupPlane {
    pub board: PresenceBoard,
    sessions: BTreeMap<AttemptId, AttemptSession>,
    grants: BTreeMap<AttemptId, HiccupGrant>,
}

/// Controller-derived topic grant. Applications cannot grant themselves named
/// topic access through their health response.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HiccupGrant {
    pub named_publish: BTreeSet<String>,
    pub named_listen: BTreeSet<String>,
}

impl HiccupPlane {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ensure_session(
        &mut self,
        attempt: AttemptId,
        workload: WorkloadId,
    ) -> &mut AttemptSession {
        self.sessions
            .entry(attempt)
            .or_insert_with(|| AttemptSession::new(workload))
    }

    pub fn remove_attempt(&mut self, attempt: AttemptId) {
        self.sessions.remove(&attempt);
        self.grants.remove(&attempt);
        self.board.remove_attempt(attempt);
    }

    pub fn set_grant(&mut self, attempt: AttemptId, grant: HiccupGrant) {
        self.grants.insert(attempt, grant);
    }

    pub fn plan(&self, attempt: AttemptId) -> Option<OutboundHealth> {
        let session = self.sessions.get(&attempt)?;
        let self_topic = gump_hiccup::CanonicalTopic::self_for(session.workload_id);
        let grant = self.grants.get(&attempt);
        Some(plan_outbound_for(session, attempt, &self.board, |topic| {
            topic == &self_topic
                || grant
                    .map(|g| g.named_listen.contains(topic.as_str()))
                    .unwrap_or(false)
        }))
    }

    pub fn on_health_ok(&mut self, ctx: HealthOkCtx<'_>) -> bool {
        let HealthOkCtx {
            placement,
            unit_id,
            attempt_id,
            fence,
            content_type,
            body,
            health_interval_ms,
            now_ms,
        } = ctx;
        let _ = self.ensure_session(attempt_id, placement.workload_id);
        let stamp = placement.stamp(unit_id, attempt_id, fence);
        let grant = self.grants.get(&attempt_id).cloned().unwrap_or_default();
        let self_topic = gump_hiccup::CanonicalTopic::self_for(placement.workload_id);
        let Self {
            board, sessions, ..
        } = self;
        let session = sessions
            .get_mut(&attempt_id)
            .expect("session inserted above");
        let out = handle_successful_health(
            session,
            board,
            HealthInbound {
                stamp,
                content_type,
                body,
                health_interval_ms,
                now: InstantMillis::from_millis(now_ms),
            },
            |topics| {
                topics
                    .publish
                    .as_ref()
                    .map(|topic| {
                        topic == &self_topic || grant.named_publish.contains(topic.as_str())
                    })
                    .unwrap_or(true)
                    && topics.listen.iter().all(|topic| {
                        topic == &self_topic || grant.named_listen.contains(topic.as_str())
                    })
            },
            |topic| topic == &self_topic || grant.named_listen.contains(topic.as_str()),
        );
        out.discovery_active
    }
}

/// Context for [`HiccupPlane::on_health_ok`].
pub struct HealthOkCtx<'a> {
    pub placement: &'a HiccupPlacement,
    pub unit_id: UnitId,
    pub attempt_id: AttemptId,
    pub fence: u64,
    pub content_type: Option<&'a str>,
    pub body: &'a [u8],
    pub health_interval_ms: u64,
    pub now_ms: u64,
}
