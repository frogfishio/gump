//! Ratatouille adapter (T01), stream capture (T02), local ring (T03),
//! authenticated keeper relay (T04).
//!
//! Authority: docs/v1/DECISIONS.md D011, docs/TELEMETRY.md, docs/v1/PROTOCOL.md §16.

#![forbid(unsafe_code)]

mod adapter;
mod identity;
mod keeper;
mod pipe_bridge;
mod plane;
mod relay;
mod ring;
mod stream;
mod topic;

pub use adapter::{
    BoundedCallbackAdapter, CallbackAdapter, MAX_RECORD_BYTES, RecordOutcome,
    SharedBoundedCallbackAdapter, SharedBoundedFnSink, SharedCallbackAdapter, SharedFnSink,
    TelemetryError, TelemetryErrorKind,
};
pub use identity::{CanonicalIdentity, NormalizedRecord, ProducerHint, TELEMETRY_PROFILE};
pub use keeper::{NodeId, RENDEZVOUS_MIN_NODES, TARGET_KEEPER_REPLICAS, select_keepers};
pub use pipe_bridge::AttemptPipeBridge;
pub use plane::{TOPIC_GUMP_LIFECYCLE, TelemetryEventView, TelemetryPlane, TelemetrySnapshot};
pub use relay::{
    BatchAuth, DEFAULT_KEEPER_SHARD_BYTES, DedupId, KeeperStore, MAX_BATCH_RECORDS, RelayError,
    RelayMesh, RelayRecord, TelemetryBatch,
};
pub use ring::{
    DEFAULT_RING_MAX_AGE, DEFAULT_RING_MAX_BYTES, GapMarker, GapReason, LocalRing, RingConfig,
    RingEvent, Subscriber, TopicFilter,
};
pub use stream::{
    BoundedRecordQueue, ChunkFlags, EmitOutcome, MAX_READ_CHUNK, MAX_STREAM_RECORD_BYTES,
    StreamCaptureError, StreamCaptureErrorKind, StreamDrain, StreamEmitter, StreamKind,
    StreamRecord, TOPIC_STDERR, TOPIC_STDOUT,
};
pub use topic::{MAX_TOPIC_LEN, TopicError, validate_topic};
