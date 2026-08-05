//! Ratatouille callback `Sink` adapter that stamps canonical identity.

use std::sync::{Arc, Mutex};

use ratatouille::Sink;
use serde::Deserialize;

use crate::identity::{CanonicalIdentity, NormalizedRecord, ProducerHint, TELEMETRY_PROFILE};
use crate::topic::{validate_topic, TopicError};

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

/// Collects Ratatouille lines and attaches authoritative Gump identity.
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
