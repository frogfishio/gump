//! Session version negotiation for `gump.cluster.v1` (PROTOCOL.md §1/§3).
//!
//! Major mismatch rejects the session. Minor is the highest version supported
//! by both peers within the advertised ranges.

use core::fmt;

use crate::pb::{EnvelopeV1, ErrorCode, HelloV1, MessageType};

/// Wire protocol major for this build.
pub const PROTOCOL_MAJOR: u32 = 1;
/// Initial wire protocol minor for this build.
pub const PROTOCOL_MINOR: u32 = 0;

/// Local support window advertised in `HelloV1`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProtocolSupport {
    pub minimum_major: u32,
    pub maximum_major: u32,
    pub minimum_minor: u32,
    pub maximum_minor: u32,
}

impl ProtocolSupport {
    /// Support window for this binary (major 1 only, minor 0..=PROTOCOL_MINOR).
    pub const fn this_build() -> Self {
        Self {
            minimum_major: PROTOCOL_MAJOR,
            maximum_major: PROTOCOL_MAJOR,
            minimum_minor: 0,
            maximum_minor: PROTOCOL_MINOR,
        }
    }

    pub fn contains_major(self, major: u32) -> bool {
        major >= self.minimum_major && major <= self.maximum_major
    }
}

/// Negotiated session parameters after both Hellos validate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NegotiatedSession {
    pub protocol_major: u32,
    pub protocol_minor: u32,
    pub maximum_control_frame: u32,
    pub maximum_bulk_chunk: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NegotiateError {
    InvalidHello(String),
    IncompatibleVersion {
        local: ProtocolSupport,
        peer_min_major: u32,
        peer_max_major: u32,
        peer_min_minor: u32,
        peer_max_minor: u32,
    },
    EnvelopeVersion {
        major: u32,
        minor: u32,
        expected_major: u32,
        max_minor: u32,
    },
    EnvelopeMessageType {
        got: i32,
    },
    EnvelopeField(String),
}

impl fmt::Display for NegotiateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHello(msg) => write!(f, "invalid hello: {msg}"),
            Self::IncompatibleVersion { .. } => write!(f, "incompatible protocol version"),
            Self::EnvelopeVersion {
                major,
                minor,
                expected_major,
                max_minor,
            } => write!(
                f,
                "envelope version {major}.{minor} outside {expected_major}.0..={max_minor}"
            ),
            Self::EnvelopeMessageType { got } => {
                write!(f, "unexpected envelope message_type {got}")
            }
            Self::EnvelopeField(msg) => write!(f, "invalid envelope: {msg}"),
        }
    }
}

impl std::error::Error for NegotiateError {}

impl NegotiateError {
    pub fn error_code(&self) -> ErrorCode {
        match self {
            Self::IncompatibleVersion { .. } | Self::EnvelopeVersion { .. } => {
                ErrorCode::IncompatibleVersion
            }
            Self::InvalidHello(_)
            | Self::EnvelopeMessageType { .. }
            | Self::EnvelopeField(_) => ErrorCode::InvalidArgument,
        }
    }
}

/// Validate a peer `HelloV1` and negotiate major/minor + frame ceilings.
pub fn negotiate_hello(
    local: ProtocolSupport,
    local_max_control: u32,
    local_max_bulk: u32,
    peer: &HelloV1,
) -> Result<NegotiatedSession, NegotiateError> {
    validate_hello(peer)?;

    if peer.minimum_major > peer.maximum_major || peer.minimum_minor > peer.maximum_minor {
        return Err(NegotiateError::InvalidHello(
            "peer hello has inverted version ranges".into(),
        ));
    }

    // Major intersection must be exactly {PROTOCOL_MAJOR} for v1 sessions.
    let major_lo = local.minimum_major.max(peer.minimum_major);
    let major_hi = local.maximum_major.min(peer.maximum_major);
    if major_lo > major_hi || major_lo != PROTOCOL_MAJOR || major_hi != PROTOCOL_MAJOR {
        return Err(NegotiateError::IncompatibleVersion {
            local,
            peer_min_major: peer.minimum_major,
            peer_max_major: peer.maximum_major,
            peer_min_minor: peer.minimum_minor,
            peer_max_minor: peer.maximum_minor,
        });
    }

    let minor_lo = local.minimum_minor.max(peer.minimum_minor);
    let minor_hi = local.maximum_minor.min(peer.maximum_minor);
    if minor_lo > minor_hi {
        return Err(NegotiateError::IncompatibleVersion {
            local,
            peer_min_major: peer.minimum_major,
            peer_max_major: peer.maximum_major,
            peer_min_minor: peer.minimum_minor,
            peer_max_minor: peer.maximum_minor,
        });
    }

    if peer.maximum_control_frame == 0 || peer.maximum_bulk_chunk == 0 {
        return Err(NegotiateError::InvalidHello(
            "peer frame ceilings must be non-zero".into(),
        ));
    }

    Ok(NegotiatedSession {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: minor_hi,
        maximum_control_frame: local_max_control.min(peer.maximum_control_frame),
        maximum_bulk_chunk: local_max_bulk.min(peer.maximum_bulk_chunk),
    })
}

/// Build the local Hello advertisement for this build.
pub fn local_hello(
    node_id: [u8; 16],
    node_incarnation: u64,
    roles: impl IntoIterator<Item = String>,
    capabilities: impl IntoIterator<Item = String>,
    connection_nonce: [u8; 16],
) -> HelloV1 {
    let support = ProtocolSupport::this_build();
    let mut roles: Vec<String> = roles.into_iter().collect();
    let mut capabilities: Vec<String> = capabilities.into_iter().collect();
    roles.sort();
    roles.dedup();
    capabilities.sort();
    capabilities.dedup();
    HelloV1 {
        node_id: node_id.to_vec(),
        node_incarnation,
        minimum_major: support.minimum_major,
        maximum_major: support.maximum_major,
        minimum_minor: support.minimum_minor,
        maximum_minor: support.maximum_minor,
        roles,
        capabilities,
        maximum_control_frame: crate::frame::MAX_CONTROL_FRAME as u32,
        maximum_bulk_chunk: 4 * 1024 * 1024,
        raft_node_id: None,
        connection_nonce: connection_nonce.to_vec(),
    }
}

fn validate_hello(hello: &HelloV1) -> Result<(), NegotiateError> {
    if hello.node_id.len() != 16 {
        return Err(NegotiateError::InvalidHello(
            "node_id must be 16 bytes".into(),
        ));
    }
    if hello.connection_nonce.len() != 16 {
        return Err(NegotiateError::InvalidHello(
            "connection_nonce must be 16 bytes".into(),
        ));
    }
    if !is_sorted_unique(&hello.roles) {
        return Err(NegotiateError::InvalidHello(
            "roles must be sorted unique".into(),
        ));
    }
    if !is_sorted_unique(&hello.capabilities) {
        return Err(NegotiateError::InvalidHello(
            "capabilities must be sorted unique".into(),
        ));
    }
    Ok(())
}

fn is_sorted_unique(items: &[String]) -> bool {
    items.windows(2).all(|w| w[0] < w[1])
}

/// Validate an inbound envelope against a negotiated session.
pub fn validate_envelope(
    session: NegotiatedSession,
    expected_cluster_id: &[u8; 16],
    envelope: &EnvelopeV1,
) -> Result<(), NegotiateError> {
    if envelope.protocol_major != session.protocol_major
        || envelope.protocol_minor > session.protocol_minor
    {
        return Err(NegotiateError::EnvelopeVersion {
            major: envelope.protocol_major,
            minor: envelope.protocol_minor,
            expected_major: session.protocol_major,
            max_minor: session.protocol_minor,
        });
    }
    if envelope.message_type == MessageType::Unspecified as i32 {
        return Err(NegotiateError::EnvelopeMessageType {
            got: envelope.message_type,
        });
    }
    if envelope.cluster_id.as_slice() != expected_cluster_id {
        return Err(NegotiateError::EnvelopeField(
            "cluster_id mismatch".into(),
        ));
    }
    for (name, bytes) in [
        ("cluster_incarnation", &envelope.cluster_incarnation),
        ("sender_node_id", &envelope.sender_node_id),
        ("message_id", &envelope.message_id),
    ] {
        if bytes.len() != 16 {
            return Err(NegotiateError::EnvelopeField(format!(
                "{name} must be 16 bytes"
            )));
        }
    }
    if let Some(id) = &envelope.correlation_id {
        if id.len() != 16 {
            return Err(NegotiateError::EnvelopeField(
                "correlation_id must be 16 bytes".into(),
            ));
        }
    }
    if let Some(id) = &envelope.operation_id {
        if id.len() != 16 {
            return Err(NegotiateError::EnvelopeField(
                "operation_id must be 16 bytes".into(),
            ));
        }
    }
    if envelope.body.is_empty() {
        return Err(NegotiateError::EnvelopeField("body must be non-empty".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::MAX_CONTROL_FRAME;

    fn peer_hello(min_maj: u32, max_maj: u32, min_min: u32, max_min: u32) -> HelloV1 {
        HelloV1 {
            node_id: vec![1; 16],
            node_incarnation: 1,
            minimum_major: min_maj,
            maximum_major: max_maj,
            minimum_minor: min_min,
            maximum_minor: max_min,
            roles: vec!["agent".into()],
            capabilities: vec![],
            maximum_control_frame: MAX_CONTROL_FRAME as u32,
            maximum_bulk_chunk: 1 << 20,
            raft_node_id: None,
            connection_nonce: vec![2; 16],
        }
    }

    #[test]
    fn compatible_peers_negotiate_minor() {
        let local = ProtocolSupport::this_build();
        let peer = peer_hello(1, 1, 0, 0);
        let n = negotiate_hello(local, MAX_CONTROL_FRAME as u32, 1 << 22, &peer).unwrap();
        assert_eq!(n.protocol_major, 1);
        assert_eq!(n.protocol_minor, 0);
    }

    #[test]
    fn major_mismatch_is_rejected() {
        let local = ProtocolSupport::this_build();
        let peer = peer_hello(2, 2, 0, 0);
        let err = negotiate_hello(local, MAX_CONTROL_FRAME as u32, 1 << 20, &peer).unwrap_err();
        assert_eq!(err.error_code(), ErrorCode::IncompatibleVersion);
    }
}
