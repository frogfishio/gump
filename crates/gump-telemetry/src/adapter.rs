//! Ratatouille callback `Sink` adapter that stamps canonical identity.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ratatouille::Sink;
use serde::Deserialize;

use crate::identity::{CanonicalIdentity, NormalizedRecord, ProducerHint, TELEMETRY_PROFILE};
use crate::ring::{DEFAULT_RING_MAX_BYTES, RingConfig};
use crate::topic::{TopicError, validate_topic};

/// Maximum formatted Ratatouille line accepted into the adapter (D011: 64 KiB).
pub const MAX_RECORD_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum TelemetryErrorKind {
    Topic,
    Oversize,
    Identity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TelemetryError {
    kind: TelemetryErrorKind,
    message: String,
}

impl TelemetryError {
    pub fn new(kind: TelemetryErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> TelemetryErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for TelemetryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for TelemetryError {}

impl From<TopicError> for TelemetryError {
    fn from(value: TopicError) -> Self {
        Self::new(TelemetryErrorKind::Topic, value.to_string())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordOutcome {
    Accepted,
    /// Accepted after dropping one or more older retained records (D011).
    AcceptedDropOldest,
    RejectedOversize,
    RejectedTopic,
}

#[derive(Debug, Deserialize)]
struct NdjsonLine {
    #[serde(default)]
    topic: Option<String>,
    #[serde(default)]
    seq: Option<u64>,
    /// Ratatouille NDJSON payload array (`args`).
    #[serde(default)]
    args: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    message: Option<String>,
    /// Ratatouille uses `src` for SourceIdentity.
    #[serde(default)]
    src: Option<NdjsonSource>,
    #[serde(default)]
    source: Option<NdjsonSource>,
}

#[derive(Debug, Deserialize)]
struct NdjsonSource {
    #[serde(default)]
    app: Option<String>,
    #[serde(default, rename = "where")]
    r#where: Option<String>,
    #[serde(default)]
    instance: Option<String>,
}

/// **Test-only** unbounded collector for Ratatouille corpus / forgery fixtures.
///
/// Production paths must use [`BoundedCallbackAdapter`] (D011 drop-oldest).
/// Do not wire this type into `gump-server` or agent supervision.
///
/// Application `src` fields are retained as [`ProducerHint`] only — they never
/// replace [`CanonicalIdentity`] (TELEMETRY.md §3 / RUNTIME.md §14).
pub struct CallbackAdapter {
    identity: CanonicalIdentity,
    local_sequence: u64,
    records: Vec<NormalizedRecord>,
    outcomes: Vec<RecordOutcome>,
}

impl CallbackAdapter {
    pub fn new(identity: CanonicalIdentity) -> Self {
        Self {
            identity,
            local_sequence: 0,
            records: Vec::new(),
            outcomes: Vec::new(),
        }
    }

    pub fn identity(&self) -> &CanonicalIdentity {
        &self.identity
    }

    pub fn records(&self) -> &[NormalizedRecord] {
        &self.records
    }

    pub fn outcomes(&self) -> &[RecordOutcome] {
        &self.outcomes
    }

    /// Ingest one already-formatted Ratatouille line (no trailing newline).
    pub fn ingest_line(&mut self, line: &str) -> RecordOutcome {
        let outcome = self.ingest_line_inner(line);
        self.outcomes.push(outcome);
        outcome
    }

    fn ingest_line_inner(&mut self, line: &str) -> RecordOutcome {
        if line.len() > MAX_RECORD_BYTES {
            return RecordOutcome::RejectedOversize;
        }

        let (topic, topic_sequence, message, producer) = match parse_line(line) {
            ParsedLine::Ndjson {
                topic,
                seq,
                message,
                producer,
            } => (topic, seq, message, producer),
            ParsedLine::Text { topic, message } => (topic, None, message, ProducerHint::default()),
            ParsedLine::Unusable => {
                return RecordOutcome::RejectedTopic;
            }
        };

        if validate_topic(&topic).is_err() {
            return RecordOutcome::RejectedTopic;
        }

        self.local_sequence = self.local_sequence.saturating_add(1);
        self.records.push(NormalizedRecord {
            profile: TELEMETRY_PROFILE,
            topic,
            topic_sequence,
            message,
            identity: self.identity.clone(),
            producer,
            local_sequence: self.local_sequence,
        });
        RecordOutcome::Accepted
    }

    pub fn into_shared(self) -> SharedCallbackAdapter {
        SharedCallbackAdapter {
            inner: Arc::new(Mutex::new(self)),
        }
    }
}

impl Sink for CallbackAdapter {
    fn write_line(&mut self, line: &str) {
        let _ = self.ingest_line(line);
    }
}

fn record_retained_bytes(rec: &NormalizedRecord) -> usize {
    // Exact owned-string charge (STL-18 / D011): no fixed overhead fudge.
    let mut n = rec.topic.len().saturating_add(rec.message.len());
    if let Some(app) = rec.producer.app.as_deref() {
        n = n.saturating_add(app.len());
    }
    if let Some(where_) = rec.producer.r#where.as_deref() {
        n = n.saturating_add(where_.len());
    }
    if let Some(instance) = rec.producer.instance.as_deref() {
        n = n.saturating_add(instance.len());
    }
    n = n.saturating_add(rec.identity.namespace.as_str().len());
    n = n.saturating_add(rec.identity.app_id.as_str().len());
    if let Some(role) = rec.identity.role.as_ref() {
        n = n.saturating_add(role.as_str().len());
    }
    n
}

/// Production Ratatouille callback sink: D011 bounded window, drop-oldest.
pub struct BoundedCallbackAdapter {
    identity: CanonicalIdentity,
    local_sequence: u64,
    records: VecDeque<NormalizedRecord>,
    inserted_at: VecDeque<Instant>,
    total_bytes: usize,
    max_bytes: usize,
    max_age: Duration,
    max_records: Option<usize>,
    accepted: u64,
    dropped_oldest: u64,
    rejected_oversize: u64,
    rejected_topic: u64,
}

impl BoundedCallbackAdapter {
    pub fn new(identity: CanonicalIdentity) -> Self {
        Self::with_config(identity, RingConfig::default())
    }

    pub fn with_config(identity: CanonicalIdentity, config: RingConfig) -> Self {
        Self {
            identity,
            local_sequence: 0,
            records: VecDeque::new(),
            inserted_at: VecDeque::new(),
            total_bytes: 0,
            max_bytes: config.max_bytes.max(1),
            max_age: config.max_age,
            // Prefer explicit test ceilings; production defaults to byte/age via max_bytes.
            max_records: config.max_records.map(|n| n.max(1)),
            accepted: 0,
            dropped_oldest: 0,
            rejected_oversize: 0,
            rejected_topic: 0,
        }
    }

    /// Convenience: bound by record count (overflow tests).
    pub fn with_max_records(identity: CanonicalIdentity, max_records: usize) -> Self {
        Self::with_config(
            identity,
            RingConfig {
                max_bytes: DEFAULT_RING_MAX_BYTES,
                max_age: crate::ring::DEFAULT_RING_MAX_AGE,
                max_records: Some(max_records),
            },
        )
    }

    pub fn identity(&self) -> &CanonicalIdentity {
        &self.identity
    }

    pub fn records(&self) -> &VecDeque<NormalizedRecord> {
        &self.records
    }

    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    pub fn accepted(&self) -> u64 {
        self.accepted
    }

    pub fn dropped_oldest(&self) -> u64 {
        self.dropped_oldest
    }

    pub fn rejected_oversize(&self) -> u64 {
        self.rejected_oversize
    }

    pub fn ingest_line(&mut self, line: &str) -> RecordOutcome {
        if line.len() > MAX_RECORD_BYTES {
            self.rejected_oversize = self.rejected_oversize.saturating_add(1);
            return RecordOutcome::RejectedOversize;
        }

        let (topic, topic_sequence, message, producer) = match parse_line(line) {
            ParsedLine::Ndjson {
                topic,
                seq,
                message,
                producer,
            } => (topic, seq, message, producer),
            ParsedLine::Text { topic, message } => (topic, None, message, ProducerHint::default()),
            ParsedLine::Unusable => {
                self.rejected_topic = self.rejected_topic.saturating_add(1);
                return RecordOutcome::RejectedTopic;
            }
        };

        if validate_topic(&topic).is_err() {
            self.rejected_topic = self.rejected_topic.saturating_add(1);
            return RecordOutcome::RejectedTopic;
        }

        let next_seq = self.local_sequence.saturating_add(1);
        let record = NormalizedRecord {
            profile: TELEMETRY_PROFILE,
            topic,
            topic_sequence,
            message,
            identity: self.identity.clone(),
            producer,
            local_sequence: next_seq,
        };
        let incoming = record_retained_bytes(&record);
        let now = Instant::now();
        self.expire_by_age(now);

        // Reject any single record that cannot fit the ceiling, even on an empty queue.
        if incoming > self.max_bytes {
            self.rejected_oversize = self.rejected_oversize.saturating_add(1);
            return RecordOutcome::RejectedOversize;
        }

        let mut dropped = false;
        while self.needs_evict(incoming) {
            if let Some(old) = self.records.pop_front() {
                let _ = self.inserted_at.pop_front();
                self.total_bytes = self.total_bytes.saturating_sub(record_retained_bytes(&old));
                self.dropped_oldest = self.dropped_oldest.saturating_add(1);
                dropped = true;
            } else {
                break;
            }
        }

        // Defensive: if eviction could not make room (should not happen after oversize check).
        if self.total_bytes.saturating_add(incoming) > self.max_bytes {
            self.rejected_oversize = self.rejected_oversize.saturating_add(1);
            return RecordOutcome::RejectedOversize;
        }

        self.local_sequence = next_seq;
        self.records.push_back(record);
        self.inserted_at.push_back(now);
        self.total_bytes = self.total_bytes.saturating_add(incoming);
        self.accepted = self.accepted.saturating_add(1);
        if dropped {
            RecordOutcome::AcceptedDropOldest
        } else {
            RecordOutcome::Accepted
        }
    }

    fn expire_by_age(&mut self, now: Instant) {
        while let Some(front_at) = self.inserted_at.front().copied() {
            if now.saturating_duration_since(front_at) <= self.max_age {
                break;
            }
            let _ = self.inserted_at.pop_front();
            if let Some(old) = self.records.pop_front() {
                self.total_bytes = self.total_bytes.saturating_sub(record_retained_bytes(&old));
                self.dropped_oldest = self.dropped_oldest.saturating_add(1);
            }
        }
    }

    fn needs_evict(&self, incoming: usize) -> bool {
        if self.records.is_empty() {
            return false;
        }
        if self.total_bytes.saturating_add(incoming) > self.max_bytes {
            return true;
        }
        if let Some(max) = self.max_records {
            if self.records.len() >= max {
                return true;
            }
        }
        false
    }

    pub fn into_shared(self) -> SharedBoundedCallbackAdapter {
        SharedBoundedCallbackAdapter {
            inner: Arc::new(Mutex::new(self)),
        }
    }
}

impl Sink for BoundedCallbackAdapter {
    fn write_line(&mut self, line: &str) {
        let _ = self.ingest_line(line);
    }
}

/// Shared production adapter for Ratatouille `Logger` sinks.
#[derive(Clone)]
pub struct SharedBoundedCallbackAdapter {
    inner: Arc<Mutex<BoundedCallbackAdapter>>,
}

impl SharedBoundedCallbackAdapter {
    pub fn records(&self) -> Vec<NormalizedRecord> {
        self.inner
            .lock()
            .expect("adapter lock")
            .records()
            .iter()
            .cloned()
            .collect()
    }

    pub fn dropped_oldest(&self) -> u64 {
        self.inner.lock().expect("adapter lock").dropped_oldest()
    }

    pub fn accepted(&self) -> u64 {
        self.inner.lock().expect("adapter lock").accepted()
    }

    pub fn as_fn_sink(&self) -> SharedBoundedFnSink {
        SharedBoundedFnSink {
            inner: self.inner.clone(),
        }
    }
}

pub struct SharedBoundedFnSink {
    inner: Arc<Mutex<BoundedCallbackAdapter>>,
}

impl Sink for SharedBoundedFnSink {
    fn write_line(&mut self, line: &str) {
        let _ = self.inner.lock().expect("adapter lock").ingest_line(line);
    }
}

/// Shared adapter usable as a Ratatouille `FnSink` / `Logger` sink target.
#[derive(Clone)]
pub struct SharedCallbackAdapter {
    inner: Arc<Mutex<CallbackAdapter>>,
}

impl SharedCallbackAdapter {
    pub fn records(&self) -> Vec<NormalizedRecord> {
        self.inner.lock().expect("adapter lock").records().to_vec()
    }

    pub fn outcomes(&self) -> Vec<RecordOutcome> {
        self.inner.lock().expect("adapter lock").outcomes().to_vec()
    }

    pub fn identity(&self) -> CanonicalIdentity {
        self.inner.lock().expect("adapter lock").identity().clone()
    }

    pub fn as_fn_sink(&self) -> SharedFnSink {
        SharedFnSink {
            inner: self.inner.clone(),
        }
    }
}

/// `Sink` wrapper that locks the shared adapter per line.
pub struct SharedFnSink {
    inner: Arc<Mutex<CallbackAdapter>>,
}

impl Sink for SharedFnSink {
    fn write_line(&mut self, line: &str) {
        let _ = self.inner.lock().expect("adapter lock").ingest_line(line);
    }
}

enum ParsedLine {
    Ndjson {
        topic: String,
        seq: Option<u64>,
        message: String,
        producer: ProducerHint,
    },
    Text {
        topic: String,
        message: String,
    },
    Unusable,
}

fn parse_line(line: &str) -> ParsedLine {
    if let Ok(parsed) = serde_json::from_str::<NdjsonLine>(line) {
        let topic = match parsed.topic {
            Some(t) if !t.is_empty() => t,
            _ => return ParsedLine::Unusable,
        };
        let message = if let Some(m) = parsed.message.or(parsed.msg) {
            m
        } else if let Some(args) = parsed.args {
            args.into_iter()
                .map(|v| match v {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                })
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            String::new()
        };
        let src = parsed.src.or(parsed.source);
        let producer = src
            .map(|s| ProducerHint {
                app: s.app,
                r#where: s.r#where,
                instance: s.instance,
            })
            .unwrap_or_default();
        return ParsedLine::Ndjson {
            topic,
            seq: parsed.seq,
            message,
            producer,
        };
    }

    let mut parts = line.splitn(2, char::is_whitespace);
    let topic = match parts.next() {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => return ParsedLine::Unusable,
    };
    let message = parts.next().unwrap_or("").to_string();
    ParsedLine::Text { topic, message }
}
