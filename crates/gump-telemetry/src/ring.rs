//! Bounded local recent-window ring and subscriber API (DELIVERY T03).
//!
//! Authority: docs/v1/RUNTIME.md §14–§15, docs/TELEMETRY.md §7/§12, D011.
//!
//! Defaults: 8 MiB or 30 seconds per attempt window (whichever binds first).
//! Overflow drops oldest and is visible as explicit gap markers to subscribers.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::stream::{EmitOutcome, StreamEmitter, StreamRecord};

/// Default per-attempt byte ceiling (D011).
pub const DEFAULT_RING_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Default per-attempt age ceiling (D011).
pub const DEFAULT_RING_MAX_AGE: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RingConfig {
    pub max_bytes: usize,
    pub max_age: Duration,
    pub max_records: Option<usize>,
}

impl Default for RingConfig {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_RING_MAX_BYTES,
            max_age: DEFAULT_RING_MAX_AGE,
            max_records: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum GapReason {
    /// Ring overflow dropped oldest retained records.
    OverflowDropOldest,
    /// Subscriber cursor lagged behind an overflow truncation.
    SubscriberLag,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GapMarker {
    pub topic: &'static str,
    /// First dropped/missing stream_sequence (inclusive).
    pub from_sequence: u64,
    /// First still-available stream_sequence after the gap (exclusive of gap).
    pub to_sequence: u64,
    pub reason: GapReason,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RingEvent {
    Record(StreamRecord),
    Gap(GapMarker),
}

#[derive(Clone, Debug)]
struct RingEntry {
    record: StreamRecord,
    inserted_at: Instant,
    bytes: usize,
}

/// In-memory bounded recent window for one attempt's captured streams.
#[derive(Debug)]
pub struct LocalRing {
    config: RingConfig,
    entries: VecDeque<RingEntry>,
    total_bytes: usize,
    /// Monotonic generation bumped on every truncation; subscribers detect lag.
    generation: u64,
    /// Next ring-global sequence assigned to pushed records (independent of
    /// per-stream `stream_sequence`).
    ring_sequence: u64,
    pushed: u64,
    dropped_oldest: u64,
}

impl LocalRing {
    pub fn new(config: RingConfig) -> Self {
        Self {
            config: RingConfig {
                max_bytes: config.max_bytes.max(1),
                max_age: config.max_age,
                max_records: config.max_records.map(|n| n.max(1)),
            },
            entries: VecDeque::new(),
            total_bytes: 0,
            generation: 0,
            ring_sequence: 0,
            pushed: 0,
            dropped_oldest: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn dropped_oldest(&self) -> u64 {
        self.dropped_oldest
    }

    pub fn pushed(&self) -> u64 {
        self.pushed
    }

    /// Push a stream record, dropping oldest as needed for byte/record/age bounds.
    pub fn push(&mut self, record: StreamRecord, now: Instant) -> EmitOutcome {
        self.expire_by_age(now);
        let bytes = record.bytes.len().saturating_add(64); // framing overhead budget
        let mut outcome = EmitOutcome::Accepted;

        // Ensure capacity for the new record.
        while self.needs_evict(bytes) {
            if self.entries.pop_front().is_some() {
                // recompute below
                self.dropped_oldest += 1;
                self.generation = self.generation.saturating_add(1);
                outcome = EmitOutcome::DroppedOldest;
            } else {
                break;
            }
            self.recompute_bytes();
        }

        // If still too large for an empty ring, accept but keep only this record
        // truncated conceptually — still store it (single record may exceed max_bytes).
        if bytes > self.config.max_bytes && !self.entries.is_empty() {
            while self.entries.pop_front().is_some() {
                self.dropped_oldest += 1;
                self.generation = self.generation.saturating_add(1);
                outcome = EmitOutcome::DroppedOldest;
            }
            self.total_bytes = 0;
        }

        self.entries.push_back(RingEntry {
            record,
            inserted_at: now,
            bytes,
        });
        self.total_bytes = self.total_bytes.saturating_add(bytes);
        self.ring_sequence = self.ring_sequence.saturating_add(1);
        self.pushed = self.pushed.saturating_add(1);
        outcome
    }

    fn needs_evict(&self, incoming: usize) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        if self.total_bytes.saturating_add(incoming) > self.config.max_bytes {
            return true;
        }
        if let Some(max) = self.config.max_records {
            if self.entries.len() >= max {
                return true;
            }
        }
        false
    }

    fn expire_by_age(&mut self, now: Instant) {
        while let Some(front) = self.entries.front() {
            if now.saturating_duration_since(front.inserted_at) <= self.config.max_age {
                break;
            }
            let _ = self.entries.pop_front();
            self.dropped_oldest += 1;
            self.generation = self.generation.saturating_add(1);
            self.recompute_bytes();
        }
    }

    fn recompute_bytes(&mut self) {
        self.total_bytes = self.entries.iter().map(|e| e.bytes).sum();
    }

    /// Open a subscriber that first replays the current window, then follows live pushes.
    pub fn subscribe(&self, filter: TopicFilter) -> Subscriber {
        Subscriber {
            filter,
            next_index: 0,
            seen_generation: self.generation,
            replay_done: false,
            last_stream_seq_by_topic: std::collections::BTreeMap::new(),
            pending_gap: None,
        }
    }
}

impl StreamEmitter for LocalRing {
    fn emit(&mut self, record: StreamRecord) -> EmitOutcome {
        self.push(record, Instant::now())
    }
}

/// Topic selection for a subscriber.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TopicFilter {
    /// Exact topics to include; empty means all topics.
    pub topics: Vec<&'static str>,
}

impl TopicFilter {
    pub fn all() -> Self {
        Self { topics: Vec::new() }
    }

    pub fn only(topics: &[&'static str]) -> Self {
        Self {
            topics: topics.to_vec(),
        }
    }

    fn allows(&self, topic: &'static str) -> bool {
        self.topics.is_empty() || self.topics.iter().any(|t| *t == topic)
    }
}

/// Cursor over a [`LocalRing`]: window replay then live tail with gap markers.
#[derive(Debug)]
pub struct Subscriber {
    filter: TopicFilter,
    next_index: usize,
    seen_generation: u64,
    replay_done: bool,
    last_stream_seq_by_topic: std::collections::BTreeMap<&'static str, u64>,
    pending_gap: Option<GapMarker>,
}

impl Subscriber {
    /// Pull the next event. Returns `None` when caught up with the ring.
    pub fn poll(&mut self, ring: &LocalRing) -> Option<RingEvent> {
        if let Some(gap) = self.pending_gap.take() {
            return Some(RingEvent::Gap(gap));
        }

        // Detect truncation behind our cursor.
        if ring.generation != self.seen_generation {
            // Ring dropped entries; clamp cursor and emit lag gap if we had progress.
            let old_gen = self.seen_generation;
            self.seen_generation = ring.generation;
            if self.next_index > 0 || self.replay_done {
                self.next_index = 0;
                // Synthesize a gap using last known per-topic sequences when possible.
                if let Some((topic, &last)) = self.last_stream_seq_by_topic.iter().next() {
                    let gap = GapMarker {
                        topic,
                        from_sequence: last.saturating_add(1),
                        to_sequence: last.saturating_add(1),
                        reason: GapReason::SubscriberLag,
                    };
                    let _ = old_gen;
                    // Continue into live after delivering gap.
                    self.pending_gap = None;
                    return Some(RingEvent::Gap(gap));
                }
            }
        }

        while self.next_index < ring.entries.len() {
            let entry = &ring.entries[self.next_index];
            self.next_index += 1;
            if !self.filter.allows(entry.record.topic) {
                continue;
            }

            // Per-topic sequence gap detection within the ring window.
            let seq = entry.record.stream_sequence;
            if let Some(prev) = self
                .last_stream_seq_by_topic
                .get(entry.record.topic)
                .copied()
            {
                if seq > prev.saturating_add(1) {
                    let gap = GapMarker {
                        topic: entry.record.topic,
                        from_sequence: prev.saturating_add(1),
                        to_sequence: seq,
                        reason: GapReason::OverflowDropOldest,
                    };
                    self.last_stream_seq_by_topic
                        .insert(entry.record.topic, seq);
                    self.pending_gap = None;
                    // Deliver gap first; stash record by rewinding index.
                    self.next_index -= 1;
                    return Some(RingEvent::Gap(gap));
                }
            }
            self.last_stream_seq_by_topic
                .insert(entry.record.topic, seq);
            self.replay_done = true;
            return Some(RingEvent::Record(entry.record.clone()));
        }
        self.replay_done = true;
        None
    }
}
