//! Authenticated QUIC transport (DELIVERY C02 / DECISIONS D001, D007).
//!
//! Owns session establishment, frame/chunk ceilings, reconnect policy, and
//! certificate-rotation drain. Protocol types stay in `gump-protocol`; this
//! crate does not leak Quinn/rustls types across its public API except where
//! tests need loopback endpoints.

#![forbid(unsafe_code)]

mod identity;
mod limits;
mod quic;
mod reconnect;
mod rotation;
mod tls;

pub use identity::{prefer_session, NodeRole, OrderingPrefer, TransportIdentity};
pub use limits::{TransportLimitError, TransportLimits};
pub use quic::{QuicEndpoint, QuicSession, TransportError};
pub use reconnect::{ReconnectDecision, ReconnectPolicy};
pub use rotation::{RotationAction, RotationPlan, SessionSlot};
pub use tls::{mint_identity, mint_identity_pair, CaBundle, IdentityMaterial, TlsBuildError};
