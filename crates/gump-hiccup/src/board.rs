//! Latest-presence keeper for local and ephemeral peer discovery (HICCUP.md §6, §11).

use std::collections::BTreeMap;

use blake3::Hasher;
use gump_types::{AttemptId, InstantMillis, NodeId, WorkloadId};

use crate::codec::{Declaration, Delivery, Introduction};
use crate::limits::{MAX_INTRODUCTIONS_PER_POST, MAX_KEEPER_BYTES, MAX_PUBLISHERS_PER_TOPIC};
use crate::stamp::{PlacementStamp, application_topic};
use crate::topic::{CanonicalTopic, ResolvedTopics};

/// A deliberately smaller ceiling than the in-process keeper. The clustered
/// exchange rides a bounded control frame and remains best-effort.
pub const MAX_CLUSTER_SNAPSHOT_BYTES: usize = 192 * 1024;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct KeeperPresenceV1 {
    topic: String,
    #[serde(default)]
    directory_visible: bool,
    cluster_id: String,
    namespace: String,
    app_id: String,
    workload_id: String,
    capsule_id: String,
    execution_id: String,
    unit_id: String,
    role: Option<String>,
    rank: Option<u32>,
    attempt_id: String,
    node_id: String,
    agent_incarnation: u64,
    placement_fence_digest: [u8; 32],
    receiver_reachable_ip: Option<String>,
    data: Option<serde_json::Value>,
    secret_data: Option<String>,
    expires_at_ms: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct KeeperSnapshotV1 {
    schema: String,
    node_id: String,
    complete: bool,
    records: Vec<KeeperPresenceV1>,
}

/// Combine driver-local snapshots into one authoritative node snapshot without
/// exposing the keeper wire representation to the server crate.
pub fn combine_cluster_snapshots(local_node: NodeId, snapshots: &[&str]) -> Result<String, String> {
    let mut records = Vec::new();
    let mut complete = true;
    for encoded in snapshots {
        if encoded.len() > MAX_CLUSTER_SNAPSHOT_BYTES {
            return Err("Hiccup cluster snapshot exceeds ceiling".into());
        }
        let snapshot: KeeperSnapshotV1 = serde_json::from_str(encoded)
            .map_err(|e| format!("decode local Hiccup snapshot: {e}"))?;
        if snapshot.schema != "gump.hiccup-keeper-snapshot/1"
            || snapshot.node_id != local_node.to_hyphenated()
        {
            return Err("local Hiccup snapshot identity mismatch".into());
        }
        complete &= snapshot.complete;
        records.extend(snapshot.records);
    }
    let mut snapshot = KeeperSnapshotV1 {
        schema: "gump.hiccup-keeper-snapshot/1".into(),
        node_id: local_node.to_hyphenated(),
        complete,
        records,
    };
    loop {
        let encoded = serde_json::to_string(&snapshot).map_err(|e| e.to_string())?;
        if encoded.len() <= MAX_CLUSTER_SNAPSHOT_BYTES {
            return Ok(encoded);
        }
        if snapshot.records.pop().is_none() {
            return Err("Hiccup cluster snapshot envelope exceeds ceiling".into());
        }
        snapshot.complete = false;
    }
}

/// Current presence for one publishing attempt on one topic.
#[derive(Clone, Debug, PartialEq)]
pub struct Presence {
    pub canonical_topic: CanonicalTopic,
    pub stamp: PlacementStamp,
    pub data: Option<serde_json::Value>,
    pub secret_data: Option<String>,
    pub expires_at: InstantMillis,
    pub content_digest: [u8; 32],
    /// True for the capability-map form; false for legacy topic presence.
    pub directory_visible: bool,
}

fn presence_size(presence: &Presence) -> usize {
    256 + presence.secret_data.as_ref().map(|s| s.len()).unwrap_or(0)
        + presence
            .data
            .as_ref()
            .and_then(|d| serde_json::to_vec(d).ok())
            .map(|b| b.len())
            .unwrap_or(0)
}

fn digest_presence(
    topic: &CanonicalTopic,
    stamp: &PlacementStamp,
    data: &Option<serde_json::Value>,
    secret: &Option<String>,
) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(topic.as_str().as_bytes());
    h.update(stamp.unit_id.as_bytes());
    h.update(stamp.attempt_id.as_bytes());
    h.update(&stamp.placement_fence_digest);
    if let Some(d) = data {
        if let Ok(b) = serde_json::to_vec(d) {
            h.update(&b);
        }
    }
    if let Some(s) = secret {
        h.update(s.as_bytes());
    }
    *h.finalize().as_bytes()
}

#[derive(Clone, Debug, Default)]
pub struct PresenceBoard {
    /// topic → attempt → presence
    by_topic: BTreeMap<CanonicalTopic, BTreeMap<AttemptId, Presence>>,
    /// attempt → topics published (for fast remove)
    by_attempt: BTreeMap<AttemptId, Vec<CanonicalTopic>>,
    approx_bytes: usize,
    /// Safe counters for diagnostics (never token/data/secret).
    pub omit_count: u64,
}

impl PresenceBoard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn approx_bytes(&self) -> usize {
        self.approx_bytes
    }

    pub fn presence_count(&self) -> usize {
        self.by_attempt.len()
    }

    pub fn publisher_count(&self, topic: &CanonicalTopic) -> usize {
        self.by_topic.get(topic).map(|m| m.len()).unwrap_or(0)
    }

    /// Return current presence for an authoritative attempt, when any.
    pub fn presence_for_attempt(&self, attempt: AttemptId) -> Option<&Presence> {
        let topic = self.by_attempt.get(&attempt)?.first()?;
        self.by_topic.get(topic)?.get(&attempt)
    }

    /// Export a bounded, credential-free snapshot for authenticated peer
    /// exchange. Tokens are session-local and never enter this representation.
    pub fn export_cluster_snapshot(
        &self,
        local_node: NodeId,
        now: InstantMillis,
    ) -> Result<String, String> {
        let mut records = Vec::new();
        let mut encoded_bytes = 128usize; // bounded envelope overhead
        let mut complete = true;
        for bucket in self.by_topic.values() {
            for presence in bucket.values() {
                if presence.expires_at <= now {
                    continue;
                }
                let stamp = &presence.stamp;
                if stamp.node_id != local_node {
                    continue;
                }
                let record = KeeperPresenceV1 {
                    topic: presence.canonical_topic.as_str().to_string(),
                    directory_visible: presence.directory_visible,
                    cluster_id: stamp.cluster_id.to_hyphenated(),
                    namespace: stamp.namespace.clone(),
                    app_id: stamp.app_id.clone(),
                    workload_id: stamp.workload_id.to_hyphenated(),
                    capsule_id: stamp.capsule_id.to_hyphenated(),
                    execution_id: stamp.execution_id.to_hyphenated(),
                    unit_id: stamp.unit_id.to_hyphenated(),
                    role: stamp.role.clone(),
                    rank: stamp.rank,
                    attempt_id: stamp.attempt_id.to_hyphenated(),
                    node_id: stamp.node_id.to_hyphenated(),
                    agent_incarnation: stamp.agent_incarnation,
                    placement_fence_digest: stamp.placement_fence_digest,
                    receiver_reachable_ip: stamp.receiver_reachable_ip.clone(),
                    data: presence.data.clone(),
                    secret_data: presence.secret_data.clone(),
                    expires_at_ms: presence.expires_at.as_millis(),
                };
                let record_bytes = serde_json::to_vec(&record)
                    .map_err(|e| e.to_string())?
                    .len();
                let separator = usize::from(!records.is_empty());
                if encoded_bytes
                    .saturating_add(separator)
                    .saturating_add(record_bytes)
                    > MAX_CLUSTER_SNAPSHOT_BYTES
                {
                    complete = false;
                    break;
                }
                encoded_bytes = encoded_bytes
                    .saturating_add(separator)
                    .saturating_add(record_bytes);
                records.push(record);
            }
            if !complete {
                break;
            }
        }
        let snapshot = KeeperSnapshotV1 {
            schema: "gump.hiccup-keeper-snapshot/1".into(),
            node_id: local_node.to_hyphenated(),
            complete,
            records,
        };
        let encoded = serde_json::to_string(&snapshot).map_err(|e| e.to_string())?;
        if encoded.len() > MAX_CLUSTER_SNAPSHOT_BYTES {
            return Err("Hiccup cluster snapshot envelope exceeds ceiling".into());
        }
        Ok(encoded)
    }

    /// Merge a peer snapshot into this node's ephemeral keeper. Every stamp is
    /// reconstructed and constrained; malformed snapshots fail as a unit.
    pub fn merge_cluster_snapshot(
        &mut self,
        snapshot: &str,
        now: InstantMillis,
    ) -> Result<usize, String> {
        if snapshot.len() > MAX_CLUSTER_SNAPSHOT_BYTES {
            return Err("Hiccup cluster snapshot exceeds ceiling".into());
        }
        let snapshot: KeeperSnapshotV1 =
            serde_json::from_str(snapshot).map_err(|e| format!("decode Hiccup snapshot: {e}"))?;
        if snapshot.schema != "gump.hiccup-keeper-snapshot/1" {
            return Err("unsupported Hiccup keeper snapshot schema".into());
        }
        let source_node: NodeId = snapshot
            .node_id
            .parse()
            .map_err(|_| "invalid Hiccup snapshot node id")?;
        let mut merged = 0usize;
        let mut refreshed_attempts = std::collections::BTreeSet::new();
        let mut incoming = Vec::new();
        for record in snapshot.records {
            if record.expires_at_ms <= now.as_millis() {
                continue;
            }
            let workload_id = record
                .workload_id
                .parse::<WorkloadId>()
                .map_err(|_| "invalid Hiccup workload id".to_string())?;
            let topic = CanonicalTopic::from_keeper(&record.topic, workload_id)
                .map_err(|e| format!("invalid Hiccup keeper topic: {e}"))?;
            let stamp = PlacementStamp {
                cluster_id: record
                    .cluster_id
                    .parse()
                    .map_err(|_| "invalid cluster id")?,
                namespace: record.namespace,
                app_id: record.app_id,
                workload_id,
                capsule_id: record
                    .capsule_id
                    .parse()
                    .map_err(|_| "invalid capsule id")?,
                execution_id: record
                    .execution_id
                    .parse()
                    .map_err(|_| "invalid execution id")?,
                unit_id: record.unit_id.parse().map_err(|_| "invalid unit id")?,
                role: record.role,
                rank: record.rank,
                attempt_id: record
                    .attempt_id
                    .parse()
                    .map_err(|_| "invalid attempt id")?,
                node_id: record.node_id.parse().map_err(|_| "invalid node id")?,
                agent_incarnation: record.agent_incarnation,
                placement_fence_digest: record.placement_fence_digest,
                health_eligible: true,
                receiver_reachable_ip: record.receiver_reachable_ip,
            };
            if stamp.node_id != source_node {
                return Err("Hiccup snapshot record/source node mismatch".into());
            }
            let presence = Presence {
                content_digest: digest_presence(&topic, &stamp, &record.data, &record.secret_data),
                canonical_topic: topic,
                stamp,
                data: record.data,
                secret_data: record.secret_data,
                expires_at: InstantMillis::from_millis(record.expires_at_ms),
                directory_visible: record.directory_visible,
            };
            incoming.push(presence);
            merged = merged.saturating_add(1);
        }

        if snapshot.complete {
            let incoming_attempts: std::collections::BTreeSet<_> = incoming
                .iter()
                .map(|presence| presence.stamp.attempt_id)
                .collect();
            let departed: Vec<_> = self
                .by_attempt
                .keys()
                .copied()
                .filter(|attempt| {
                    self.presence_for_attempt(*attempt)
                        .map(|presence| {
                            presence.stamp.node_id == source_node
                                && !incoming_attempts.contains(attempt)
                        })
                        .unwrap_or(false)
                })
                .collect();
            for attempt in departed {
                self.remove_attempt(attempt);
            }
        }
        for presence in incoming {
            if refreshed_attempts.insert(presence.stamp.attempt_id) {
                self.remove_attempt(presence.stamp.attempt_id);
            }
            self.insert_presence(presence);
        }
        Ok(merged)
    }

    /// Replace prior declaration for this attempt completely (latest-presence).
    pub fn upsert(
        &mut self,
        resolved: &ResolvedTopics,
        stamp: PlacementStamp,
        decl: &Declaration,
        expires_at: InstantMillis,
    ) {
        self.remove_attempt(stamp.attempt_id);
        let Some(topic) = &resolved.publish else {
            return;
        };
        if !stamp.health_eligible {
            return;
        }
        let presence = Presence {
            content_digest: digest_presence(topic, &stamp, &decl.data, &decl.secret_data),
            canonical_topic: topic.clone(),
            stamp,
            data: decl.data.clone(),
            secret_data: decl.secret_data.clone(),
            expires_at,
            directory_visible: false,
        };
        self.insert_presence(presence);
    }

    /// Replace the complete capability advertisement for an attempt. An empty
    /// map is a valid directory consumer with no advertised capability.
    pub fn upsert_capabilities(
        &mut self,
        stamp: PlacementStamp,
        capabilities: &BTreeMap<String, serde_json::Value>,
        expires_at: InstantMillis,
    ) -> Result<(), crate::topic::TopicError> {
        self.remove_attempt(stamp.attempt_id);
        if !stamp.health_eligible {
            return Ok(());
        }
        for (name, data) in capabilities {
            if name == "@self" {
                return Err(crate::topic::TopicError::InvalidShape);
            }
            let topic = crate::topic::canonicalize_topic(name, stamp.workload_id)?;
            let presence = Presence {
                content_digest: digest_presence(&topic, &stamp, &Some(data.clone()), &None),
                canonical_topic: topic,
                stamp: stamp.clone(),
                data: Some(data.clone()),
                secret_data: None,
                expires_at,
                directory_visible: true,
            };
            self.insert_presence(presence);
        }
        Ok(())
    }

    fn insert_presence(&mut self, presence: Presence) {
        let topic = presence.canonical_topic.clone();
        let attempt = presence.stamp.attempt_id;
        let bucket = self.by_topic.entry(topic.clone()).or_default();
        if bucket.len() >= MAX_PUBLISHERS_PER_TOPIC && !bucket.contains_key(&attempt) {
            self.omit_count = self.omit_count.saturating_add(1);
            return;
        }
        let old_size = bucket.get(&attempt).map(presence_size).unwrap_or(0);
        let size_est = presence_size(&presence);
        let projected = self
            .approx_bytes
            .saturating_sub(old_size)
            .saturating_add(size_est);
        if projected > MAX_KEEPER_BYTES {
            self.omit_count = self.omit_count.saturating_add(1);
            return;
        }
        bucket.insert(attempt, presence);
        let topics = self.by_attempt.entry(attempt).or_default();
        if !topics.contains(&topic) {
            topics.push(topic);
        }
        self.approx_bytes = projected;
    }

    pub fn remove_attempt(&mut self, attempt: AttemptId) {
        if let Some(topics) = self.by_attempt.remove(&attempt) {
            for t in topics {
                if let Some(bucket) = self.by_topic.get_mut(&t) {
                    if let Some(p) = bucket.remove(&attempt) {
                        let size_est = presence_size(&p);
                        self.approx_bytes = self.approx_bytes.saturating_sub(size_est);
                    }
                    if bucket.is_empty() {
                        self.by_topic.remove(&t);
                    }
                }
            }
        }
    }

    /// Fenced / ended attempts lose presence immediately.
    pub fn fence_attempt(&mut self, attempt: AttemptId, fence_digest: &[u8; 32]) {
        let matches = self.by_attempt.get(&attempt).and_then(|topics| {
            topics.first().and_then(|t| {
                self.by_topic
                    .get(t)
                    .and_then(|b| b.get(&attempt))
                    .map(|p| p.stamp.fence_matches(fence_digest))
            })
        });
        if matches.unwrap_or(true) {
            self.remove_attempt(attempt);
        }
    }

    pub fn expire(&mut self, now: InstantMillis) {
        let mut dead = Vec::new();
        for bucket in self.by_topic.values() {
            for (id, p) in bucket {
                if p.expires_at <= now {
                    dead.push(*id);
                }
            }
        }
        for id in dead {
            self.remove_attempt(id);
        }
    }

    /// Build a rotating delivery for a listener. Never includes the listener's own attempt.
    pub fn deliver(
        &self,
        listen: &[CanonicalTopic],
        listener_attempt: AttemptId,
        listener_workload: WorkloadId,
        rotation_offset: usize,
        authorize_topic: impl Fn(&CanonicalTopic) -> bool,
    ) -> (Delivery, usize) {
        let mut matched: Vec<&Presence> = Vec::new();
        for topic in listen {
            if !authorize_topic(topic) {
                continue;
            }
            if let Some(bucket) = self.by_topic.get(topic) {
                for p in bucket.values() {
                    if p.stamp.attempt_id == listener_attempt {
                        continue;
                    }
                    matched.push(p);
                }
            }
        }
        matched.sort_by_key(|p| (p.canonical_topic.as_str().to_string(), p.stamp.attempt_id));
        self.delivery_from_matches(matched, listener_workload, rotation_offset)
    }

    /// Build a rotating view of every current advertised capability. Unknown
    /// capability names and data remain opaque to Gump and are filtered by the
    /// receiving application.
    pub fn deliver_directory(
        &self,
        listener_attempt: AttemptId,
        listener_workload: WorkloadId,
        rotation_offset: usize,
    ) -> (Delivery, usize) {
        let mut matched: Vec<&Presence> = self
            .by_topic
            .values()
            .flat_map(|bucket| bucket.values())
            .filter(|presence| {
                presence.directory_visible && presence.stamp.attempt_id != listener_attempt
            })
            .collect();
        matched.sort_by_key(|presence| {
            (
                presence.canonical_topic.as_str().to_string(),
                presence.stamp.attempt_id,
            )
        });
        self.delivery_from_matches(matched, listener_workload, rotation_offset)
    }

    fn delivery_from_matches(
        &self,
        matched: Vec<&Presence>,
        listener_workload: WorkloadId,
        rotation_offset: usize,
    ) -> (Delivery, usize) {
        let total = matched.len();
        let start = if total == 0 {
            0
        } else {
            rotation_offset % total
        };
        let mut messages = Vec::new();
        for i in 0..total.min(MAX_INTRODUCTIONS_PER_POST) {
            let presence = matched[(start + i) % total];
            let capabilities = if presence.directory_visible {
                BTreeMap::from([(
                    application_topic(&presence.canonical_topic, listener_workload),
                    presence
                        .data
                        .clone()
                        .unwrap_or_else(|| serde_json::json!({})),
                )])
            } else {
                BTreeMap::new()
            };
            let introduction = Introduction {
                topic: application_topic(&presence.canonical_topic, listener_workload),
                from: presence.stamp.public_from(),
                capabilities,
                data: presence.data.clone(),
                secret_data: presence.secret_data.clone(),
            };
            messages.push(introduction);
            let candidate = Delivery {
                hiccup: 1,
                messages: messages.clone(),
                more: true,
            };
            if serde_json::to_vec(&candidate)
                .map(|bytes| bytes.len() > crate::limits::MAX_DELIVERY_BYTES)
                .unwrap_or(true)
            {
                messages.pop();
                break;
            }
        }
        let delivered = messages.len();
        let next_offset = if total == 0 {
            0
        } else {
            (start + delivered) % total
        };
        (
            Delivery {
                hiccup: 1,
                messages,
                more: total > delivered,
            },
            next_offset,
        )
    }
}

#[cfg(test)]
mod cluster_snapshot_tests {
    use super::*;
    use gump_types::{CapsuleId, ClusterId, ExecutionId, NodeId, UnitId};

    fn stamp(node_id: NodeId, workload_id: WorkloadId) -> PlacementStamp {
        PlacementStamp {
            cluster_id: ClusterId::new(),
            namespace: "default".into(),
            app_id: "peer-test".into(),
            workload_id,
            capsule_id: CapsuleId::new(),
            execution_id: ExecutionId::new(),
            unit_id: UnitId::new(),
            role: None,
            rank: None,
            attempt_id: AttemptId::new(),
            node_id,
            agent_incarnation: 1,
            placement_fence_digest: [7; 32],
            health_eligible: true,
            receiver_reachable_ip: Some("10.0.0.2".into()),
        }
    }

    #[test]
    fn snapshot_exports_only_local_node_and_merges_ephemerally() {
        let local_node = NodeId::new();
        let remote_node = NodeId::new();
        let workload = WorkloadId::new();
        let topic = CanonicalTopic::self_for(workload);
        let resolved = ResolvedTopics {
            publish: Some(topic.clone()),
            listen: vec![topic.clone()],
        };
        let declaration = Declaration {
            topic: None,
            listen: None,
            data: Some(serde_json::json!({"protocol": "kismet-cluster/1"})),
            secret_data: None,
            capabilities: None,
        };
        let mut source = PresenceBoard::new();
        let local = stamp(local_node, workload);
        let remote = stamp(remote_node, workload);
        source.upsert(
            &resolved,
            local.clone(),
            &declaration,
            InstantMillis::from_millis(60_000),
        );
        source.upsert(
            &resolved,
            remote,
            &declaration,
            InstantMillis::from_millis(60_000),
        );

        let snapshot = source
            .export_cluster_snapshot(local_node, InstantMillis::from_millis(1_000))
            .expect("export");
        let mut destination = PresenceBoard::new();
        assert_eq!(
            destination
                .merge_cluster_snapshot(&snapshot, InstantMillis::from_millis(1_000))
                .expect("merge"),
            1
        );
        assert!(destination.presence_for_attempt(local.attempt_id).is_some());
        assert_eq!(destination.presence_count(), 1);
        destination.expire(InstantMillis::from_millis(60_000));
        assert_eq!(destination.presence_count(), 0);
    }

    #[test]
    fn snapshot_preserves_every_capability_for_one_attempt() {
        let node = NodeId::new();
        let workload = WorkloadId::new();
        let advertised = stamp(node, workload);
        let attempt = advertised.attempt_id;
        let capabilities = BTreeMap::from([
            (
                "kismet.cluster/1".to_string(),
                serde_json::json!({"port": 7600}),
            ),
            (
                "kismet.ingress/1".to_string(),
                serde_json::json!({"port": 443}),
            ),
        ]);
        let mut source = PresenceBoard::new();
        source
            .upsert_capabilities(
                advertised,
                &capabilities,
                InstantMillis::from_millis(60_000),
            )
            .expect("capabilities");

        let snapshot = source
            .export_cluster_snapshot(node, InstantMillis::from_millis(1_000))
            .expect("export");
        let mut destination = PresenceBoard::new();
        assert_eq!(
            destination
                .merge_cluster_snapshot(&snapshot, InstantMillis::from_millis(1_000))
                .expect("merge"),
            2
        );
        assert_eq!(destination.presence_count(), 1);
        let (directory, _) = destination.deliver_directory(AttemptId::new(), WorkloadId::new(), 0);
        assert_eq!(directory.messages.len(), 2);
        assert!(
            directory
                .messages
                .iter()
                .all(|entry| entry.from.attempt == attempt.to_string())
        );
        assert!(
            directory
                .messages
                .iter()
                .all(|entry| entry.capabilities.get(&entry.topic) == entry.data.as_ref())
        );
    }

    #[test]
    fn complete_snapshot_removes_departed_attempts_from_that_node() {
        let node = NodeId::new();
        let workload = WorkloadId::new();
        let topic = CanonicalTopic::self_for(workload);
        let resolved = ResolvedTopics {
            publish: Some(topic.clone()),
            listen: vec![topic],
        };
        let declaration = Declaration {
            topic: None,
            listen: None,
            data: Some(serde_json::json!({"protocol": "http.origin/1"})),
            secret_data: None,
            capabilities: None,
        };
        let mut source = PresenceBoard::new();
        let old = stamp(node, workload);
        source.upsert(
            &resolved,
            old.clone(),
            &declaration,
            InstantMillis::from_millis(60_000),
        );
        let first = source
            .export_cluster_snapshot(node, InstantMillis::from_millis(1_000))
            .expect("first export");
        let mut destination = PresenceBoard::new();
        destination
            .merge_cluster_snapshot(&first, InstantMillis::from_millis(1_000))
            .expect("first merge");

        source.remove_attempt(old.attempt_id);
        let mut replacement = stamp(node, workload);
        replacement.unit_id = old.unit_id;
        source.upsert(
            &resolved,
            replacement.clone(),
            &declaration,
            InstantMillis::from_millis(60_000),
        );
        let second = source
            .export_cluster_snapshot(node, InstantMillis::from_millis(2_000))
            .expect("replacement export");
        destination
            .merge_cluster_snapshot(&second, InstantMillis::from_millis(2_000))
            .expect("replacement merge");

        assert!(destination.presence_for_attempt(old.attempt_id).is_none());
        assert!(
            destination
                .presence_for_attempt(replacement.attempt_id)
                .is_some()
        );
        assert_eq!(destination.presence_count(), 1);
    }

    #[test]
    fn directory_page_respects_encoded_byte_ceiling() {
        let node = NodeId::new();
        let workload = WorkloadId::new();
        let mut board = PresenceBoard::new();
        for index in 0..40 {
            let capabilities = BTreeMap::from([(
                format!("bulk/provider-{index}"),
                serde_json::json!({"padding": "x".repeat(7_500)}),
            )]);
            board
                .upsert_capabilities(
                    stamp(node, workload),
                    &capabilities,
                    InstantMillis::from_millis(60_000),
                )
                .expect("capability");
        }
        let (page, _) = board.deliver_directory(AttemptId::new(), WorkloadId::new(), 0);
        let encoded = serde_json::to_vec(&page).expect("encode delivery");
        assert!(encoded.len() <= crate::limits::MAX_DELIVERY_BYTES);
        assert!(page.more);
        assert!(page.messages.len() < 40);
    }
}
