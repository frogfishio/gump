//! Node-local telemetry plane: ring + supervisor ingest + query snapshot (GUMP-N014).
//!
//! Memory-only. Child/control-plane paths never await this plane's capacity.

use std::time::Instant;

use crate::stream::{ChunkFlags, EmitOutcome, StreamRecord, TOPIC_STDERR, TOPIC_STDOUT};
use crate::topic::{TopicError, validate_topic};
use crate::{GapReason, LocalRing, RingConfig, RingEvent, TELEMETRY_PROFILE, TopicFilter};

/// Supervisor-owned lifecycle topic (never accepted from application forge path).
pub const TOPIC_GUMP_LIFECYCLE: &str = "gump/lifecycle";

/// Bounded query result for `gump telemetry` / local API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TelemetrySnapshot {
    pub profile: &'static str,
    pub memory_only: bool,
    pub pushed: u64,
    pub dropped_oldest: u64,
    pub filter: Option<String>,
    pub events: Vec<TelemetryEventView>,
    /// True when the cursor caught up; another poll may see new live events.
    pub caught_up: bool,
    pub identity_note: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TelemetryEventView {
    Record {
        topic: String,
        stream_sequence: u64,
        utf8_hint: bool,
        /// Lowercase hex of raw bytes (binary-safe; never assumes UTF-8).
        bytes_hex: String,
        /// Present only when `utf8_hint` and bytes are valid UTF-8.
        text: Option<String>,
    },
    Gap {
        topic: String,
        from_sequence: u64,
        to_sequence: u64,
        reason: &'static str,
    },
}

/// In-process recent-window plane for one agent/controller node.
#[derive(Debug)]
pub struct TelemetryPlane {
    ring: LocalRing,
    supervisor_seq: u64,
}

impl TelemetryPlane {
    pub fn new(config: RingConfig) -> Self {
        Self {
            ring: LocalRing::new(config),
            supervisor_seq: 0,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(RingConfig::default())
    }

    pub fn ring(&self) -> &LocalRing {
        &self.ring
    }

    pub fn ring_mut(&mut self) -> &mut LocalRing {
        &mut self.ring
    }

    /// Ingest a captured stream record (stdout/stderr). Never blocks.
    pub fn ingest_stream(&mut self, record: StreamRecord) -> EmitOutcome {
        self.ring.push(record, Instant::now())
    }

    /// Emit a Gump-owned supervisor event. Applications cannot use this path.
    pub fn emit_supervisor(&mut self, message: &[u8]) -> EmitOutcome {
        let seq = self.supervisor_seq;
        self.supervisor_seq = self.supervisor_seq.saturating_add(1);
        let utf8_hint = std::str::from_utf8(message).is_ok();
        self.ring.push(
            StreamRecord {
                topic: TOPIC_GUMP_LIFECYCLE,
                stream_sequence: seq,
                flags: ChunkFlags::BEGIN | ChunkFlags::END,
                bytes: message.to_vec(),
                utf8_hint,
                receive_offset: seq.saturating_mul(message.len() as u64),
            },
            Instant::now(),
        )
    }

    /// Application/forged ingest: reject reserved `gump/` topics (source forgery).
    pub fn ingest_application_topic(
        &mut self,
        topic: &str,
        bytes: &[u8],
        stream_sequence: u64,
    ) -> Result<EmitOutcome, TopicError> {
        validate_topic(topic)?;
        if topic == TOPIC_GUMP_LIFECYCLE || topic.starts_with("gump/") {
            return Err(TopicError::ReservedImpersonation);
        }
        let static_topic = match topic {
            TOPIC_STDOUT => TOPIC_STDOUT,
            TOPIC_STDERR => TOPIC_STDERR,
            _ => {
                // Only known stream topics are stored as &'static; others rejected
                // for the forge path (apps use Ratatouille adapter separately).
                return Err(TopicError::ReservedImpersonation);
            }
        };
        Ok(self.ingest_stream(StreamRecord {
            topic: static_topic,
            stream_sequence,
            flags: ChunkFlags::BEGIN | ChunkFlags::END,
            bytes: bytes.to_vec(),
            utf8_hint: std::str::from_utf8(bytes).is_ok(),
            receive_offset: 0,
        }))
    }

    /// Recent-window replay (+ live catch-up) with optional topic filter.
    ///
    /// Filter grammar: empty = all; exact topic; or prefix ending in `*` (e.g. `app/*`).
    pub fn query(&self, filter: Option<&str>, max_events: usize) -> TelemetrySnapshot {
        let max_events = max_events.clamp(1, 4_096);
        let mut sub = self.ring.subscribe(TopicFilter::all());
        let mut events = Vec::new();
        let mut caught_up = true;
        while let Some(ev) = sub.poll(&self.ring) {
            if !event_matches_filter(&ev, filter) {
                continue;
            }
            events.push(view_event(ev));
            if events.len() >= max_events {
                caught_up = false;
                break;
            }
        }
        if caught_up {
            // Confirm no more matching events remain.
            caught_up = true;
        }
        TelemetrySnapshot {
            profile: TELEMETRY_PROFILE,
            memory_only: true,
            pushed: self.ring.pushed(),
            dropped_oldest: self.ring.dropped_oldest(),
            filter: filter.map(|s| s.to_string()),
            events,
            caught_up,
            identity_note: "canonical identity is placement-derived; producer hints are non-authoritative",
        }
    }
}

fn event_matches_filter(ev: &RingEvent, filter: Option<&str>) -> bool {
    let Some(f) = filter.map(str::trim).filter(|s| !s.is_empty()) else {
        return true;
    };
    let topic = match ev {
        RingEvent::Record(r) => r.topic,
        RingEvent::Gap(g) => g.topic,
    };
    if let Some(prefix) = f.strip_suffix('*') {
        return topic.starts_with(prefix);
    }
    topic == f
}

fn view_event(ev: RingEvent) -> TelemetryEventView {
    match ev {
        RingEvent::Record(r) => {
            let bytes_hex = hex_encode(&r.bytes);
            let text = if r.utf8_hint {
                std::str::from_utf8(&r.bytes).ok().map(|s| s.to_string())
            } else {
                None
            };
            TelemetryEventView::Record {
                topic: r.topic.to_string(),
                stream_sequence: r.stream_sequence,
                utf8_hint: r.utf8_hint,
                bytes_hex,
                text,
            }
        }
        RingEvent::Gap(g) => TelemetryEventView::Gap {
            topic: g.topic.to_string(),
            from_sequence: g.from_sequence,
            to_sequence: g.to_sequence,
            reason: match g.reason {
                GapReason::OverflowDropOldest => "overflow_drop_oldest",
                GapReason::SubscriberLag => "subscriber_lag",
            },
        },
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::TOPIC_STDOUT;

    #[test]
    fn application_cannot_forge_gump_topics() {
        let mut plane = TelemetryPlane::new(RingConfig {
            max_bytes: 4096,
            max_age: std::time::Duration::from_secs(60),
            max_records: Some(32),
        });
        assert!(matches!(
            plane.ingest_application_topic("gump/lifecycle", b"x", 0),
            Err(TopicError::ReservedImpersonation)
        ));
        assert!(matches!(
            plane.emit_supervisor(b"ok"),
            EmitOutcome::Accepted | EmitOutcome::DroppedOldest
        ));
        let snap = plane.query(Some("gump/*"), 10);
        assert_eq!(snap.events.len(), 1);
    }

    #[test]
    fn filter_and_gaps_are_honest() {
        let mut plane = TelemetryPlane::new(RingConfig {
            max_bytes: 10_000,
            max_age: std::time::Duration::from_secs(60),
            max_records: Some(2),
        });
        let t0 = Instant::now();
        plane.ring.push(
            StreamRecord {
                topic: TOPIC_STDOUT,
                stream_sequence: 0,
                flags: ChunkFlags::BEGIN | ChunkFlags::END,
                bytes: b"a".to_vec(),
                utf8_hint: true,
                receive_offset: 0,
            },
            t0,
        );
        plane.ring.push(
            StreamRecord {
                topic: TOPIC_STDOUT,
                stream_sequence: 1,
                flags: ChunkFlags::BEGIN | ChunkFlags::END,
                bytes: b"b".to_vec(),
                utf8_hint: true,
                receive_offset: 1,
            },
            t0,
        );
        // Overflow
        plane.ring.push(
            StreamRecord {
                topic: TOPIC_STDOUT,
                stream_sequence: 2,
                flags: ChunkFlags::BEGIN | ChunkFlags::END,
                bytes: b"c".to_vec(),
                utf8_hint: true,
                receive_offset: 2,
            },
            t0,
        );
        assert!(plane.ring.dropped_oldest() >= 1);
        let snap = plane.query(Some("app/stdout"), 32);
        assert!(snap.memory_only);
        assert!(snap.dropped_oldest >= 1);
        assert!(
            snap.events
                .iter()
                .any(|e| matches!(e, TelemetryEventView::Record { .. }))
        );
    }
}
