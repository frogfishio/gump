//! Ratatouille adapter, identity (T01), and stdout/stderr capture (T02).
//!
//! Authority: docs/v1/DECISIONS.md D011, docs/v1/RUNTIME.md §14, docs/TELEMETRY.md §3.
//! Application-supplied Ratatouille source fields are producer hints only.

#![forbid(unsafe_code)]

mod adapter;
mod identity;
mod ring;
mod stream;
mod topic;

pub use adapter::{
    CallbackAdapter, RecordOutcome, SharedCallbackAdapter, SharedFnSink, TelemetryError,
    TelemetryErrorKind, MAX_RECORD_BYTES,
};
pub use identity::{CanonicalIdentity, NormalizedRecord, ProducerHint, TELEMETRY_PROFILE};
pub use stream::{
    BoundedRecordQueue, ChunkFlags, EmitOutcome, StreamCaptureError, StreamCaptureErrorKind,
    StreamDrain, StreamEmitter, StreamKind, StreamRecord, MAX_READ_CHUNK, MAX_STREAM_RECORD_BYTES,
    TOPIC_STDERR, TOPIC_STDOUT,
};
pub use ring::{
    GapMarker, GapReason, LocalRing, RingConfig, RingEvent, Subscriber, TopicFilter,
    DEFAULT_RING_MAX_AGE, DEFAULT_RING_MAX_BYTES,
};
pub use topic::{validate_topic, TopicError, MAX_TOPIC_LEN};
