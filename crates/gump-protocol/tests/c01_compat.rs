//! C01 exit evidence: wire goldens + version/envelope compatibility.
//!
//! Authority: docs/v1/DELIVERY.md C01, docs/v1/PROTOCOL.md §1–§3,
//! `proto/gump/v1/cluster.proto`.

use gump_protocol::goldens::{
    encode_envelope, encode_error, encode_hello, sample_envelope, sample_error, sample_hello,
};
use gump_protocol::pb::{EnvelopeV1, ErrorCode, HelloV1, MessageType};
use gump_protocol::{
    FrameKind, MAX_CONTROL_FRAME, MAX_HELLO_FRAME, NegotiateError, PROTOCOL_MAJOR, PROTOCOL_MINOR,
    ProtocolSupport, local_hello, negotiate_hello, validate_envelope,
};
use prost::Message;

#[test]
fn local_hello_advertises_this_build() {
    let hello = local_hello([9; 16], 1, ["memory".into(), "agent".into()], [], [8; 16]);
    assert_eq!(hello.minimum_major, PROTOCOL_MAJOR);
    assert_eq!(hello.maximum_major, PROTOCOL_MAJOR);
    assert_eq!(hello.minimum_minor, 0);
    assert_eq!(hello.maximum_minor, PROTOCOL_MINOR);
    assert_eq!(hello.roles, vec!["agent".to_string(), "memory".to_string()]);
    assert_eq!(hello.maximum_control_frame, MAX_CONTROL_FRAME as u32);
}

#[test]
fn negotiate_rejects_disjoint_major() {
    let local = ProtocolSupport::this_build();
    let mut peer = sample_hello();
    peer.minimum_major = 2;
    peer.maximum_major = 2;
    let err = negotiate_hello(local, MAX_CONTROL_FRAME as u32, 1 << 20, &peer).unwrap_err();
    assert!(matches!(err, NegotiateError::IncompatibleVersion { .. }));
    assert_eq!(err.error_code(), ErrorCode::IncompatibleVersion);
}

#[test]
fn negotiate_rejects_unsorted_roles() {
    let local = ProtocolSupport::this_build();
    let mut peer = sample_hello();
    peer.roles = vec!["memory".into(), "agent".into()];
    let err = negotiate_hello(local, MAX_CONTROL_FRAME as u32, 1 << 20, &peer).unwrap_err();
    assert!(matches!(err, NegotiateError::InvalidHello(_)));
}

#[test]
fn negotiate_takes_min_frame_ceilings() {
    let local = ProtocolSupport::this_build();
    let mut peer = sample_hello();
    peer.maximum_control_frame = 32 * 1024;
    peer.maximum_bulk_chunk = 64 * 1024;
    let n = negotiate_hello(local, MAX_CONTROL_FRAME as u32, 4 * 1024 * 1024, &peer).unwrap();
    assert_eq!(n.maximum_control_frame, 32 * 1024);
    assert_eq!(n.maximum_bulk_chunk, 64 * 1024);
}

#[test]
fn envelope_accepts_negotiated_version() {
    let local = ProtocolSupport::this_build();
    let session =
        negotiate_hello(local, MAX_CONTROL_FRAME as u32, 1 << 20, &sample_hello()).unwrap();
    let body = encode_error(&sample_error());
    let env = sample_envelope(body);
    let cluster: [u8; 16] = {
        let mut id = [0u8; 16];
        id.copy_from_slice(&env.cluster_id);
        id
    };
    validate_envelope(session, &cluster, &env).unwrap();
}

#[test]
fn envelope_rejects_future_minor_and_wrong_cluster() {
    let local = ProtocolSupport::this_build();
    let session =
        negotiate_hello(local, MAX_CONTROL_FRAME as u32, 1 << 20, &sample_hello()).unwrap();
    let body = encode_error(&sample_error());
    let mut env = sample_envelope(body);
    env.protocol_minor = 9;
    let cluster: [u8; 16] = {
        let mut id = [0u8; 16];
        id.copy_from_slice(&env.cluster_id);
        id
    };
    assert!(matches!(
        validate_envelope(session, &cluster, &env),
        Err(NegotiateError::EnvelopeVersion { .. })
    ));

    env.protocol_minor = 0;
    env.cluster_id = vec![0xff; 16];
    assert!(matches!(
        validate_envelope(session, &cluster, &env),
        Err(NegotiateError::EnvelopeField(_))
    ));
}

#[test]
fn hello_wire_fits_hello_frame_ceiling() {
    let hello_bytes = encode_hello(&sample_hello());
    assert!(hello_bytes.len() <= MAX_HELLO_FRAME);
    let frame = gump_protocol::encode_frame(&hello_bytes, FrameKind::Hello).unwrap();
    let (payload, _) = gump_protocol::frame::split_frame(&frame, FrameKind::Hello).unwrap();
    assert_eq!(payload, hello_bytes.as_slice());
    let hello = HelloV1::decode(hello_bytes.as_slice()).unwrap();
    assert_eq!(hello.maximum_major, 1);
    let env = EnvelopeV1::decode(
        encode_envelope(&sample_envelope(encode_error(&sample_error()))).as_slice(),
    )
    .unwrap();
    assert_eq!(env.message_type, MessageType::Response as i32);
}

#[test]
fn peer_with_wider_minor_still_lands_on_zero() {
    // Local only speaks minor 0; peer claims 0..=5 → negotiate 0.
    let local = ProtocolSupport::this_build();
    let mut peer = sample_hello();
    peer.maximum_minor = 5;
    let n = negotiate_hello(local, MAX_CONTROL_FRAME as u32, 1 << 20, &peer).unwrap();
    assert_eq!(n.protocol_minor, 0);
}
