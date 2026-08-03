//! Exact encode/decode fixtures for W03.
//!
//! Fixtures live under `testdata/goldens/`. Set `GUMP_WRITE_GOLDENS=1` when
//! regenerating after an intentional schema change.

use prost::Message;

use crate::pb::{
    AppIdentityV1, EnvelopeV1, ErrorCode, ErrorV1, MessageType, NamedValueV1, RetryClass,
};

/// Deterministic ErrorV1 used as the primary RPC golden.
pub fn sample_error() -> ErrorV1 {
    ErrorV1 {
        code: ErrorCode::Unauthorized as i32,
        reason: "policy.denied".into(),
        safe_message: "action denied".into(),
        field_path: Some("deploy.declaration".into()),
        retry_class: RetryClass::Never as i32,
        retry_after_ms: None,
        details: vec![NamedValueV1 {
            name: "policy_decision_id".into(),
            value: "pd-001".into(),
        }],
    }
}

/// Deterministic EnvelopeV1 wrapping the sample error body.
pub fn sample_envelope(body: Vec<u8>) -> EnvelopeV1 {
    EnvelopeV1 {
        protocol_major: 1,
        protocol_minor: 0,
        message_type: MessageType::Response as i32,
        cluster_id: bytes16(1),
        cluster_incarnation: bytes16(2),
        sender_node_id: bytes16(3),
        sender_incarnation: 7,
        message_id: bytes16(4),
        correlation_id: Some(bytes16(5)),
        operation_id: Some(bytes16(6)),
        sent_unix_ms: 1_700_000_000_000,
        body,
    }
}

pub fn sample_app_identity() -> AppIdentityV1 {
    AppIdentityV1 {
        namespace: "prod".into(),
        app_id: "accounts".into(),
        workload_id: Some(bytes16(9)),
        description: Some("accounts service".into()),
        version_annotation: Some("1.2.3".into()),
    }
}

fn bytes16(seed: u8) -> Vec<u8> {
    let mut out = vec![0u8; 16];
    out[0] = seed;
    out[15] = seed ^ 0xff;
    out
}

/// Encode helpers used by tests and later conformance tickets.
pub fn encode_error(msg: &ErrorV1) -> Vec<u8> {
    msg.encode_to_vec()
}

pub fn encode_envelope(msg: &EnvelopeV1) -> Vec<u8> {
    msg.encode_to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{FrameKind, encode_frame, split_frame};
    use std::fs;
    use std::path::{Path, PathBuf};

    fn testdata_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/goldens")
    }

    fn fixture_path(name: &str) -> PathBuf {
        testdata_dir().join(name)
    }

    fn read_or_write(name: &str, bytes: &[u8]) -> Vec<u8> {
        let path = fixture_path(name);
        if std::env::var_os("GUMP_WRITE_GOLDENS").is_some() {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, bytes).unwrap();
            return bytes.to_vec();
        }
        fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "missing golden {} ({e}); re-run with GUMP_WRITE_GOLDENS=1 to create",
                path.display()
            )
        })
    }

    #[test]
    fn error_v1_golden_round_trip() {
        let msg = sample_error();
        let encoded = encode_error(&msg);
        let golden = read_or_write("error_v1.bin", &encoded);
        assert_eq!(encoded, golden, "ErrorV1 encoding drifted from golden");
        let decoded = ErrorV1::decode(golden.as_slice()).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn envelope_v1_golden_round_trip() {
        let body = encode_error(&sample_error());
        let msg = sample_envelope(body);
        let encoded = encode_envelope(&msg);
        let golden = read_or_write("envelope_v1.bin", &encoded);
        assert_eq!(encoded, golden, "EnvelopeV1 encoding drifted from golden");
        let decoded = EnvelopeV1::decode(golden.as_slice()).unwrap();
        assert_eq!(decoded, msg);
        let inner = ErrorV1::decode(decoded.body.as_slice()).unwrap();
        assert_eq!(inner, sample_error());
    }

    #[test]
    fn app_identity_v1_golden_round_trip() {
        let msg = sample_app_identity();
        let encoded = msg.encode_to_vec();
        let golden = read_or_write("app_identity_v1.bin", &encoded);
        assert_eq!(
            encoded, golden,
            "AppIdentityV1 encoding drifted from golden"
        );
        let decoded = AppIdentityV1::decode(golden.as_slice()).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn framed_error_respects_error_ceiling() {
        let payload = encode_error(&sample_error());
        let frame = encode_frame(&payload, FrameKind::Error).unwrap();
        let (got, _) = split_frame(&frame, FrameKind::Error).unwrap();
        assert_eq!(got, payload.as_slice());
    }
}
