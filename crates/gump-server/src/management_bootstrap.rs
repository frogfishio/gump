//! Minimal incarnation-scoped mTLS management proof used by zero-to-one bootstrap.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, CertificateSigningRequestParams,
    DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
};
use rustls::RootCertStore;
use rustls_pki_types::{
    CertificateDer, CertificateSigningRequestDer, PrivateKeyDer, PrivatePkcs8KeyDer,
};

use crate::bootstrap::InitializationResult;

pub struct ManagementAuthority {
    ca_key: KeyPair,
    ca_certificate: Certificate,
    ca_der: Vec<u8>,
    server_tls: Arc<rustls::ServerConfig>,
}

impl ManagementAuthority {
    pub fn generate() -> Result<Self, String> {
        let ca_key = KeyPair::generate().map_err(|e| format!("generate management CA key: {e}"))?;
        let mut ca_parameters = CertificateParams::new(Vec::<String>::new())
            .map_err(|e| format!("management CA parameters: {e}"))?;
        ca_parameters.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let mut ca_name = DistinguishedName::new();
        ca_name.push(DnType::CommonName, "gump-management-incarnation");
        ca_parameters.distinguished_name = ca_name;
        let ca_certificate = ca_parameters
            .self_signed(&ca_key)
            .map_err(|e| format!("generate management CA: {e}"))?;
        let ca_der = ca_certificate.der().to_vec();

        let server_key =
            KeyPair::generate().map_err(|e| format!("generate management server key: {e}"))?;
        let mut server_parameters = CertificateParams::new(vec!["localhost".into()])
            .map_err(|e| format!("management server parameters: {e}"))?;
        server_parameters.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let mut server_name = DistinguishedName::new();
        server_name.push(DnType::CommonName, "gump-management-server");
        server_parameters.distinguished_name = server_name;
        let server_certificate = server_parameters
            .signed_by(&server_key, &ca_certificate, &ca_key)
            .map_err(|e| format!("issue management server certificate: {e}"))?;

        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from(ca_der.clone()))
            .map_err(|e| format!("install management client trust: {e}"))?;
        let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
            .build()
            .map_err(|e| format!("build management client verifier: {e}"))?;
        let mut server_tls = rustls::ServerConfig::builder()
            .with_client_cert_verifier(verifier)
            .with_single_cert(
                vec![CertificateDer::from(server_certificate.der().to_vec())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key.serialize_der())),
            )
            .map_err(|e| format!("build management TLS server: {e}"))?;
        server_tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Self {
            ca_key,
            ca_certificate,
            ca_der,
            server_tls: Arc::new(server_tls),
        })
    }

    pub fn issue_client(&self, csr_base64: &str) -> Result<Vec<u8>, String> {
        let csr_der = base64::engine::general_purpose::STANDARD
            .decode(csr_base64)
            .map_err(|_| "management client CSR is invalid base64".to_string())?;
        if csr_der.is_empty() || csr_der.len() > 12 * 1024 {
            return Err("management client CSR is empty or oversized".into());
        }
        let csr =
            CertificateSigningRequestParams::from_der(&CertificateSigningRequestDer::from(csr_der))
                .map_err(|e| format!("validate management client CSR: {e}"))?;
        if csr.params.is_ca != IsCa::NoCa {
            return Err("management client CSR requests CA authority".into());
        }
        let certificate = csr
            .signed_by(&self.ca_certificate, &self.ca_key)
            .map_err(|e| format!("issue management client certificate: {e}"))?;
        Ok(certificate.der().to_vec())
    }

    pub fn ca_der(&self) -> &[u8] {
        &self.ca_der
    }

    pub fn server_tls(&self) -> Arc<rustls::ServerConfig> {
        Arc::clone(&self.server_tls)
    }
}

pub struct ManagementBootstrapEndpoint {
    listener: TcpListener,
    tls: Arc<rustls::ServerConfig>,
    session_id: String,
    result: InitializationResult,
    cluster_identity: String,
    node_identity: String,
}

impl ManagementBootstrapEndpoint {
    pub fn bind(
        bind: &str,
        authority: &ManagementAuthority,
        session_id: String,
        result: InitializationResult,
        cluster_identity: String,
        node_identity: String,
    ) -> Result<Self, String> {
        let listener =
            TcpListener::bind(bind).map_err(|e| format!("bind management endpoint: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("configure management endpoint: {e}"))?;
        Ok(Self {
            listener,
            tls: authority.server_tls(),
            session_id,
            result,
            cluster_identity,
            node_identity,
        })
    }

    pub fn serve(self) -> Result<(), String> {
        loop {
            let (mut tcp, _) = match self.listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                Err(error) => return Err(format!("accept management connection: {error}")),
            };
            tcp.set_nonblocking(false).ok();
            tcp.set_read_timeout(Some(Duration::from_secs(10))).ok();
            tcp.set_write_timeout(Some(Duration::from_secs(10))).ok();
            let mut connection = rustls::ServerConnection::new(Arc::clone(&self.tls))
                .map_err(|e| format!("create management TLS connection: {e}"))?;
            let mut stream = rustls::Stream::new(&mut connection, &mut tcp);
            if let Err(_error) = self.handle(&mut stream) {
                let _ = write_response(
                    &mut stream,
                    400,
                    &serde_json::json!({"schema":"gump.management-error/1","code":"INVALID_REQUEST"}),
                );
            }
        }
    }

    fn handle(&self, stream: &mut (impl Read + Write)) -> Result<(), String> {
        let mut bytes = Vec::with_capacity(1024);
        loop {
            if bytes.len() >= 8 * 1024 {
                return Err("management request headers exceed limit".into());
            }
            let mut chunk = [0_u8; 512];
            let read = stream
                .read(&mut chunk)
                .map_err(|e| format!("read management request: {e}"))?;
            if read == 0 {
                return Err("management request ended early".into());
            }
            bytes.extend_from_slice(&chunk[..read]);
            if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let request = std::str::from_utf8(&bytes).map_err(|_| "management request is not UTF-8")?;
        let line = request.split("\r\n").next().unwrap_or_default();
        let verification_path = format!("GET /v1/bootstrap/verify/{} HTTP/1.1", self.session_id);
        if line == verification_path {
            let (lock, ready) = &*self.result;
            let mut slot = lock.lock().map_err(|_| "bootstrap result poisoned")?;
            let result = slot
                .as_mut()
                .and_then(|result| result.as_mut().ok())
                .ok_or("bootstrap result is unavailable")?;
            if result.bootstrap_closed {
                return Err("bootstrap verification route is closed".into());
            }
            result.management_mtls_verified = true;
            ready.notify_all();
        } else if line != "GET /v1/status HTTP/1.1" {
            return Err("management route not found".into());
        }
        write_response(
            stream,
            200,
            &serde_json::json!({
                "schema":"gump.management-status/1",
                "clusterIdentity":self.cluster_identity,
                "nodeIdentity":self.node_identity,
                "status":"healthy"
            }),
        )
    }
}

fn write_response(
    stream: &mut impl Write,
    status: u16,
    body: &impl serde::Serialize,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(body).map_err(|e| e.to_string())?;
    let reason = if status == 200 { "OK" } else { "Bad Request" };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        bytes.len()
    )
    .map_err(|e| e.to_string())?;
    stream.write_all(&bytes).map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())
}
