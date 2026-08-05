//! Ratatouille adapter (T01), stream capture (T02), local ring (T03),
//! authenticated keeper relay (T04).
//!
//! Authority: docs/v1/DECISIONS.md D011, docs/TELEMETRY.md, docs/v1/PROTOCOL.md §16.

#![forbid(unsafe_code)]

mod adapter;
mod identity;
mod keeper;
mod relay;
mod ring;
mod stream;
mod topic;

pub use adapter::{
    CallbackAdapter, RecordOutcome, SharedCallbackAdapter, SharedFnSink, TelemetryError,
    TelemetryErrorKind, MAX_RECORD_BYTES,
};
pub use identity::{CanonicalIdentity, NormalizedRecord, ProducerHint, TELEMETRY_PROFILE};
pub use keeper::{
    select_keepers, NodeId, RENDEZVOUS_MIN_NODES, TARGET_KEEPER_REPLICAS,
};
pub use relay::{
    BatchAuth, DedupId, KeeperStore, RelayError, RelayMesh, RelayRecord, TelemetryBatch,
    DEFAULT_KEEPER_SHARD_BYTES, MAX_BATCH_RECORDS,
};
pub use ring::{
    GapMarker, GapReason, LocalRing, RingConfig, RingEvent, Subscriber, TopicFilter,
    DEFAULT_RING_MAX_AGE, DEFAULT_RING_MAX_BYTES,
};
pub use stream::{
    BoundedRecordQueue, ChunkFlags, EmitOutcome, StreamCaptureError, StreamCaptureErrorKind,
    StreamDrain, StreamEmitter, StreamKind, StreamRecord, MAX_READ_CHUNK, MAX_STREAM_RECORD_BYTES,
    TOPIC_STDERR, TOPIC_STDOUT,
};
pub use topic::{validate_topic, TopicError, MAX_TOPIC_LEN};
