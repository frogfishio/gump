//! Cluster-backed local Unix API types + CLI client (C08 / GUMP-N006).

pub mod client;
pub mod framing;
pub mod machine;

pub use client::{LocalClient, LocalClientError};
pub use framing::{FrameError, MAX_FRAME_BYTES, read_frame, write_frame};
pub use machine::{
    ErrorBody, LocalCall, LocalRequest, LocalResponse, MachineOutputV1, PROTOCOL_MAJOR,
    PROTOCOL_MINOR, StatusBody, TelemetryEventBody, cancelled_error, deadline_exceeded_error,
    protocol_mismatch_error, sample_cluster_admin, sample_deploy, sample_explain,
    sample_hello_response, sample_lifecycle, sample_observe, sample_recovery, sample_status,
    sample_telemetry, unauthorized_error,
};
