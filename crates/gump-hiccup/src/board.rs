//! Latest-presence board for one-node (and local keeper) discovery (HICCUP.md §6, §11).

use std::collections::BTreeMap;

use blake3::Hasher;
use gump_types::{AttemptId, InstantMillis, WorkloadId};

use crate::codec::{Declaration, Delivery, Introduction};
use crate::limits::{MAX_INTRODUCTIONS_PER_POST, MAX_KEEPER_BYTES, MAX_PUBLISHERS_PER_TOPIC};
use crate::stamp::{PlacementStamp, application_topic};
use crate::topic::{CanonicalTopic, ResolvedTopics};

/// Current presence for one publishing attempt on one topic.
#[derive(Clone, Debug, PartialEq)]
pub struct Presence {
    pub canonical_topic: CanonicalTopic,
    pub stamp: PlacementStamp,
    pub data: Option<serde_json::Value>,
    pub secret_data: Option<String>,
    pub expires_at: InstantMillis,
    pub content_digest: [u8; 32],
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

    pub fn publisher_count(&self, topic: &CanonicalTopic) -> usize {
        self.by_topic.get(topic).map(|m| m.len()).unwrap_or(0)
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
        let bucket = self.by_topic.entry(topic.clone()).or_default();
        if bucket.len() >= MAX_PUBLISHERS_PER_TOPIC && !bucket.contains_key(&stamp.attempt_id) {
            self.omit_count = self.omit_count.saturating_add(1);
            return;
        }
        let presence = Presence {
            content_digest: digest_presence(topic, &stamp, &decl.data, &decl.secret_data),
            canonical_topic: topic.clone(),
            stamp,
            data: decl.data.clone(),
            secret_data: decl.secret_data.clone(),
            expires_at,
        };
        let size_est = 256
            + presence.secret_data.as_ref().map(|s| s.len()).unwrap_or(0)
            + presence
                .data
                .as_ref()
                .and_then(|d| serde_json::to_vec(d).ok())
                .map(|b| b.len())
                .unwrap_or(0);
        if self.approx_bytes.saturating_add(size_est) > MAX_KEEPER_BYTES {
            self.omit_count = self.omit_count.saturating_add(1);
            return;
        }
        let attempt = presence.stamp.attempt_id;
        bucket.insert(attempt, presence);
        self.by_attempt.insert(attempt, vec![topic.clone()]);
        self.approx_bytes = self.approx_bytes.saturating_add(size_est);
    }

    pub fn remove_attempt(&mut self, attempt: AttemptId) {
        if let Some(topics) = self.by_attempt.remove(&attempt) {
            for t in topics {
                if let Some(bucket) = self.by_topic.get_mut(&t) {
                    if let Some(p) = bucket.remove(&attempt) {
                        let size_est = 256
                            + p.secret_data.as_ref().map(|s| s.len()).unwrap_or(0)
                            + p.data
                                .as_ref()
                                .and_then(|d| serde_json::to_vec(d).ok())
                                .map(|b| b.len())
                                .unwrap_or(0);
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
        let total = matched.len();
        let more = total > MAX_INTRODUCTIONS_PER_POST;
        let start = if total == 0 {
            0
        } else {
            rotation_offset % total
        };
        let mut messages = Vec::new();
        for i in 0..total.min(MAX_INTRODUCTIONS_PER_POST) {
            let p = matched[(start + i) % total];
            messages.push(Introduction {
                topic: application_topic(&p.canonical_topic, listener_workload),
                from: p.stamp.public_from(),
                data: p.data.clone(),
                secret_data: p.secret_data.clone(),
            });
        }
        let next_offset = if total == 0 {
            0
        } else {
            (start + messages.len()) % total
        };
        (
            Delivery {
                hiccup: 1,
                messages,
                more,
            },
            next_offset,
        )
    }
}
