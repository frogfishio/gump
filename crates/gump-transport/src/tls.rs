//! Ephemeral rustls identity material for QUIC mTLS (C02 / D007).

use core::fmt;

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use x509_parser::prelude::*;

use crate::identity::{IdentityParseError, TransportIdentity};

/// Local key + certificate chain for a node (never persisted to disk in v1).
pub struct IdentityMaterial {
    pub identity: TransportIdentity,
    pub cert_der: CertificateDer<'static>,
    pub key_der: PrivateKeyDer<'static>,
    pub ca_cert_der: CertificateDer<'static>,
}

/// Trust anchors for verifying peer node certificates.
#[derive(Clone, Debug)]
pub struct CaBundle {
    pub cert_der: CertificateDer<'static>,
}

#[derive(Clone, Debug)]
pub enum TlsBuildError {
    Rcgen(String),
    Rustls(String),
    Identity(IdentityParseError),
    X509(String),
}

impl fmt::Display for TlsBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rcgen(m) | Self::Rustls(m) | Self::X509(m) => write!(f, "{m}"),
            Self::Identity(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for TlsBuildError {}

/// Issue a short-lived node cert under a fresh local CA (test/local until S05).
pub fn mint_identity(
    identity: TransportIdentity,
) -> Result<(IdentityMaterial, CaBundle), TlsBuildError> {
    let ca_key = KeyPair::generate().map_err(|e| TlsBuildError::Rcgen(e.to_string()))?;
    let mut ca_params = CertificateParams::new(Vec::<String>::new())
        .map_err(|e| TlsBuildError::Rcgen(e.to_string()))?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let mut ca_dn = DistinguishedName::new();
    ca_dn.push(DnType::CommonName, "gump-test-ca");
    ca_params.distinguished_name = ca_dn;
    let ca_cert = ca_params
        .self_signed(&ca_key)
        .map_err(|e| TlsBuildError::Rcgen(e.to_string()))?;

    let node_key = KeyPair::generate().map_err(|e| TlsBuildError::Rcgen(e.to_string()))?;
    let sans = identity.san_names();
    let mut params =
        CertificateParams::new(sans).map_err(|e| TlsBuildError::Rcgen(e.to_string()))?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, identity.node_id.to_hyphenated());
    params.distinguished_name = dn;
    let cert = params
        .signed_by(&node_key, &ca_cert, &ca_key)
        .map_err(|e| TlsBuildError::Rcgen(e.to_string()))?;

    let material = IdentityMaterial {
        identity,
        cert_der: CertificateDer::from(cert.der().to_vec()),
        key_der: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(node_key.serialize_der())),
        ca_cert_der: CertificateDer::from(ca_cert.der().to_vec()),
    };
    let ca = CaBundle {
        cert_der: material.ca_cert_der.clone(),
    };
    Ok((material, ca))
}

impl IdentityMaterial {
    pub fn server_config(&self, trust: &CaBundle) -> Result<ServerConfig, TlsBuildError> {
        let mut roots = RootCertStore::empty();
        roots
            .add(trust.cert_der.clone())
            .map_err(|e| TlsBuildError::Rustls(e.to_string()))?;

        let mut cfg = ServerConfig::builder()
            .with_client_cert_verifier(
                rustls::server::WebPkiClientVerifier::builder(std::sync::Arc::new(roots))
                    .build()
                    .map_err(|e| TlsBuildError::Rustls(e.to_string()))?,
            )
            .with_single_cert(vec![self.cert_der.clone()], clone_key(&self.key_der))
            .map_err(|e| TlsBuildError::Rustls(e.to_string()))?;
        cfg.alpn_protocols = vec![b"gump.cluster.v1".to_vec()];
        Ok(cfg)
    }

    pub fn client_config(&self, trust: &CaBundle) -> Result<ClientConfig, TlsBuildError> {
        let mut roots = RootCertStore::empty();
        roots
            .add(trust.cert_der.clone())
            .map_err(|e| TlsBuildError::Rustls(e.to_string()))?;

        let mut cfg = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_client_auth_cert(vec![self.cert_der.clone()], clone_key(&self.key_der))
            .map_err(|e| TlsBuildError::Rustls(e.to_string()))?;
        cfg.alpn_protocols = vec![b"gump.cluster.v1".to_vec()];
        // Identity SANs are not hostnames; loopback uses dangerous ServerName.
        cfg.enable_sni = false;
        Ok(cfg)
    }
}

fn clone_key(key: &PrivateKeyDer<'static>) -> PrivateKeyDer<'static> {
    match key {
        PrivateKeyDer::Pkcs8(k) => {
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(k.secret_pkcs8_der().to_vec()))
        }
        PrivateKeyDer::Sec1(k) => PrivateKeyDer::Sec1(rustls_pki_types::PrivateSec1KeyDer::from(
            k.secret_sec1_der().to_vec(),
        )),
        PrivateKeyDer::Pkcs1(k) => PrivateKeyDer::Pkcs1(rustls_pki_types::PrivatePkcs1KeyDer::from(
            k.secret_pkcs1_der().to_vec(),
        )),
        _ => panic!("unsupported private key encoding"),
    }
}

/// Issue two node certs under one shared CA (loopback mTLS pairs).
pub fn mint_identity_pair(
    a: TransportIdentity,
    b: TransportIdentity,
) -> Result<(IdentityMaterial, IdentityMaterial, CaBundle), TlsBuildError> {
    let ca_key = KeyPair::generate().map_err(|e| TlsBuildError::Rcgen(e.to_string()))?;
    let mut ca_params = CertificateParams::new(Vec::<String>::new())
        .map_err(|e| TlsBuildError::Rcgen(e.to_string()))?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let mut ca_dn = DistinguishedName::new();
    ca_dn.push(DnType::CommonName, "gump-test-ca");
    ca_params.distinguished_name = ca_dn;
    let ca_cert = ca_params
        .self_signed(&ca_key)
        .map_err(|e| TlsBuildError::Rcgen(e.to_string()))?;
    let ca_der = CertificateDer::from(ca_cert.der().to_vec());
    let ca = CaBundle {
        cert_der: ca_der.clone(),
    };

    let mint_one = |identity: TransportIdentity| -> Result<IdentityMaterial, TlsBuildError> {
        let node_key = KeyPair::generate().map_err(|e| TlsBuildError::Rcgen(e.to_string()))?;
        let mut params = CertificateParams::new(identity.san_names())
            .map_err(|e| TlsBuildError::Rcgen(e.to_string()))?;
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, identity.node_id.to_hyphenated());
        params.distinguished_name = dn;
        let cert = params
            .signed_by(&node_key, &ca_cert, &ca_key)
            .map_err(|e| TlsBuildError::Rcgen(e.to_string()))?;
        Ok(IdentityMaterial {
            identity,
            cert_der: CertificateDer::from(cert.der().to_vec()),
            key_der: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(node_key.serialize_der())),
            ca_cert_der: ca_der.clone(),
        })
    };

    Ok((mint_one(a)?, mint_one(b)?, ca))
}

/// Extract peer identity from the leaf certificate's DNS SANs.
pub fn identity_from_cert(cert: &CertificateDer<'_>) -> Result<TransportIdentity, TlsBuildError> {
    let (_, x509) = X509Certificate::from_der(cert.as_ref())
        .map_err(|e| TlsBuildError::X509(e.to_string()))?;
    let mut names = Vec::new();
    if let Ok(Some(san)) = x509.subject_alternative_name() {
        for name in &san.value.general_names {
            if let x509_parser::extensions::GeneralName::DNSName(dns) = name {
                names.push((*dns).to_string());
            }
        }
    }
    TransportIdentity::from_san_names(&names).map_err(TlsBuildError::Identity)
}
