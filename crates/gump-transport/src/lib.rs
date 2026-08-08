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

pub use identity::{NodeRole, OrderingPrefer, TransportIdentity, prefer_session};
pub use limits::{TransportLimitError, TransportLimits};
pub use quic::{QuicEndpoint, QuicSession, TransportError};
pub use reconnect::{ReconnectDecision, ReconnectPolicy};
pub use rotation::{RotationAction, RotationPlan, SessionSlot};
pub use tls::{CaBundle, IdentityMaterial, TlsBuildError, mint_identity, mint_identity_pair};
