//! Quinn endpoint helpers wrapping rustls mTLS (D001 / D007).

use core::fmt;
use std::net::SocketAddr;
use std::sync::Arc;

use quinn::{ClientConfig as QuinnClientConfig, Endpoint, RecvStream, SendStream, ServerConfig};

use crate::identity::TransportIdentity;
use crate::limits::{TransportLimitError, TransportLimits};
use crate::tls::{identity_from_cert, CaBundle, IdentityMaterial, TlsBuildError};

#[derive(Debug)]
pub enum TransportError {
    Tls(TlsBuildError),
    Io(std::io::Error),
    QuinnConnect(String),
    QuinnAccept(String),
    PeerIdentity(String),
    Limit(TransportLimitError),
    Closed,
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tls(e) => write!(f, "tls: {e}"),
            Self::Io(e) => write!(f, "io: {e}"),
            Self::QuinnConnect(e) | Self::QuinnAccept(e) | Self::PeerIdentity(e) => {
                write!(f, "{e}")
            }
            Self::Limit(e) => write!(f, "{e}"),
            Self::Closed => write!(f, "connection closed"),
        }
    }
}

impl std::error::Error for TransportError {}

/// Bound Quinn endpoint with local identity.
pub struct QuicEndpoint {
    endpoint: Endpoint,
    pub local_identity: TransportIdentity,
    pub limits: TransportLimits,
}

/// Authenticated bidirectional session after mTLS.
#[derive(Debug)]
pub struct QuicSession {
    conn: quinn::Connection,
    pub peer: TransportIdentity,
    pub local: TransportIdentity,
    pub limits: TransportLimits,
}

impl QuicEndpoint {
    pub fn server(
        material: &IdentityMaterial,
        trust: &CaBundle,
        bind: SocketAddr,
        limits: TransportLimits,
    ) -> Result<Self, TransportError> {
        let server_crypto = material
            .server_config(trust)
            .map_err(TransportError::Tls)?;
        let mut server_config = ServerConfig::with_crypto(Arc::new(
            quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)
                .map_err(|e| TransportError::QuinnAccept(e.to_string()))?,
        ));
        // Keep defaults; application enforces frame ceilings before alloc.
        let _ = &mut server_config;

        let endpoint = Endpoint::server(server_config, bind).map_err(TransportError::Io)?;
        Ok(Self {
            endpoint,
            local_identity: material.identity.clone(),
            limits,
        })
    }

    pub fn client(
        material: &IdentityMaterial,
        trust: &CaBundle,
        bind: SocketAddr,
        limits: TransportLimits,
    ) -> Result<Self, TransportError> {
        let client_crypto = material
            .client_config(trust)
            .map_err(TransportError::Tls)?;
        let mut endpoint = Endpoint::client(bind).map_err(TransportError::Io)?;
        let quinn_client = QuinnClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)
                .map_err(|e| TransportError::QuinnConnect(e.to_string()))?,
        ));
        endpoint.set_default_client_config(quinn_client);
        Ok(Self {
            endpoint,
            local_identity: material.identity.clone(),
            limits,
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, TransportError> {
        self.endpoint.local_addr().map_err(TransportError::Io)
    }

    pub async fn accept(&self) -> Result<QuicSession, TransportError> {
        let incoming = self
            .endpoint
            .accept()
            .await
            .ok_or(TransportError::Closed)?;
        let conn = incoming
            .await
            .map_err(|e| TransportError::QuinnAccept(e.to_string()))?;
        self.session_from_conn(conn)
    }

    pub async fn connect(&self, addr: SocketAddr) -> Result<QuicSession, TransportError> {
        let conn = self
            .endpoint
            .connect(addr, "localhost")
            .map_err(|e| TransportError::QuinnConnect(e.to_string()))?
            .await
            .map_err(|e| TransportError::QuinnConnect(e.to_string()))?;
        self.session_from_conn(conn)
    }

    fn session_from_conn(&self, conn: quinn::Connection) -> Result<QuicSession, TransportError> {
        let peer = peer_identity(&conn)?;
        if peer.cluster_id != self.local_identity.cluster_id {
            return Err(TransportError::PeerIdentity(
                "peer cluster_id mismatch".into(),
            ));
        }
        Ok(QuicSession {
            conn,
            peer,
            local: self.local_identity.clone(),
            limits: self.limits,
        })
    }

    pub fn close(&self) {
        self.endpoint.close(0u32.into(), b"shutdown");
    }
}

impl QuicSession {
    pub async fn open_bi(&self) -> Result<(SendStream, RecvStream), TransportError> {
        self.conn
            .open_bi()
            .await
            .map_err(|e| TransportError::QuinnConnect(e.to_string()))
    }

    pub async fn accept_bi(&self) -> Result<(SendStream, RecvStream), TransportError> {
        self.conn
            .accept_bi()
            .await
            .map_err(|e| TransportError::QuinnAccept(e.to_string()))
    }

    /// Send a length-checked control payload (ceiling applied before write).
    pub async fn send_control(
        &self,
        send: &mut SendStream,
        payload: &[u8],
    ) -> Result<(), TransportError> {
        self.limits
            .check_control(payload.len())
            .map_err(TransportError::Limit)?;
        let len = (payload.len() as u32).to_be_bytes();
        send.write_all(&len)
            .await
            .map_err(|e| TransportError::Io(std::io::Error::other(e.to_string())))?;
        send.write_all(payload)
            .await
            .map_err(|e| TransportError::Io(std::io::Error::other(e.to_string())))?;
        Ok(())
    }

    /// Receive a length-checked control payload (reject oversize before body alloc).
    pub async fn recv_control(&self, recv: &mut RecvStream) -> Result<Vec<u8>, TransportError> {
        let mut len_buf = [0u8; 4];
        recv.read_exact(&mut len_buf)
            .await
            .map_err(|e| TransportError::Io(std::io::Error::other(e.to_string())))?;
        let len = u32::from_be_bytes(len_buf) as usize;
        self.limits
            .check_control(len)
            .map_err(TransportError::Limit)?;
        let mut body = vec![0u8; len];
        recv.read_exact(&mut body)
            .await
            .map_err(|e| TransportError::Io(std::io::Error::other(e.to_string())))?;
        Ok(body)
    }

    pub fn close(&self) {
        self.conn.close(0u32.into(), b"bye");
    }
}

fn peer_identity(conn: &quinn::Connection) -> Result<TransportIdentity, TransportError> {
    let iids = conn.peer_identity().ok_or_else(|| {
        TransportError::PeerIdentity("missing peer identity after mTLS".into())
    })?;
    let certs = iids
        .downcast_ref::<Vec<rustls::pki_types::CertificateDer<'static>>>()
        .ok_or_else(|| TransportError::PeerIdentity("unexpected peer identity type".into()))?;
    let leaf = certs
        .first()
        .ok_or_else(|| TransportError::PeerIdentity("empty peer cert chain".into()))?;
    identity_from_cert(leaf).map_err(TransportError::Tls)
}
