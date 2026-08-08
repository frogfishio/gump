//! Re-export shared local machine protocol from `gump-cli` (GUMP-N006).

pub use gump_cli::{
    ErrorBody, LocalCall, LocalRequest, LocalResponse, MachineOutputV1, PROTOCOL_MAJOR,
    PROTOCOL_MINOR, StatusBody, TelemetryEventBody, cancelled_error, deadline_exceeded_error,
    protocol_mismatch_error, sample_cluster_admin, sample_deploy, sample_explain,
    sample_hello_response, sample_lifecycle, sample_observe, sample_recovery, sample_status,
    sample_telemetry, unauthorized_error,
};
