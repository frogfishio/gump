//! Ratatouille capture adapter and `gump.ratatouille/1` identity (DELIVERY T01).
//!
//! Authority: docs/v1/DECISIONS.md D011, docs/v1/RUNTIME.md §14, docs/TELEMETRY.md §3.
//! Application-supplied Ratatouille source fields are producer hints only.

#![forbid(unsafe_code)]

mod adapter;
mod identity;
mod topic;

pub use adapter::{
    CallbackAdapter, RecordOutcome, SharedCallbackAdapter, SharedFnSink, TelemetryError,
    TelemetryErrorKind, MAX_RECORD_BYTES,
};
pub use identity::{CanonicalIdentity, NormalizedRecord, ProducerHint, TELEMETRY_PROFILE};
pub use topic::{validate_topic, TopicError, MAX_TOPIC_LEN};
