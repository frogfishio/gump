//! Pinned client for the restricted zero-to-one bootstrap endpoint.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use gump_protocol::bootstrap::{
    BootstrapHandoff, BootstrapResult, INITIALIZE_SCHEMA, InitializeRequest, MAX_HANDOFF_BYTES,
    MAX_INITIALIZE_BYTES, MAX_RESPONSE_BYTES, transcript_digest, validate_endpoint_identity,
};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use x509_parser::prelude::FromDer as _;
use zeroize::Zeroize as _;

#[derive(Clone, Debug)]
pub struct BootstrapInitializeOptions {
    pub handoff_fd: i32,
    pub activation_fd: i32,
    pub initialization_fd: i32,
    pub management_output_fd: i32,
    pub management_identity_ref: String,
    pub deadline: Duration,
}

pub fn initialize_from_handoff(
    options: BootstrapInitializeOptions,
) -> Result<BootstrapResult, String> {
    let handoff_bytes = read_fd_bounded(options.handoff_fd, MAX_HANDOFF_BYTES, "handoff")?;
    let handoff: BootstrapHandoff = serde_json::from_slice(&handoff_bytes)
        .map_err(|e| format!("invalid bootstrap handoff: {e}"))?;
    handoff.validate(OffsetDateTime::now_utc())?;

    let mut activation = read_fd_bounded(options.activation_fd, 256, "activation secret")?;
    trim_ascii_whitespace(&mut activation);
    if !(32..=256).contains(&activation.len()) {
        activation.zeroize();
        return Err("activation descriptor must contain 32..=256 bytes".into());
    }
    let mut initialization = read_fd_bounded(
        options.initialization_fd,
        MAX_INITIALIZE_BYTES / 2,
        "initialization parameters",
    )?;
    let server_parameters: serde_json::Value = serde_json::from_slice(&initialization)
        .map_err(|e| format!("invalid initialization parameters: {e}"))?;
    initialization.zeroize();
    if !server_parameters.is_object() {
        activation.zeroize();
        return Err("initialization parameters must be a JSON object".into());
    }
    let activation_code = String::from_utf8(activation.clone())
        .map_err(|_| "activation descriptor is not UTF-8".to_string())?;
    activation.zeroize();
    let management_key =
        rcgen::KeyPair::generate().map_err(|e| format!("generate management client key: {e}"))?;
    let mut management_parameters = rcgen::CertificateParams::new(vec!["gump-operator".into()])
        .map_err(|e| format!("build management client request: {e}"))?;
    management_parameters.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
    let management_csr = management_parameters
        .serialize_request(&management_key)
        .map_err(|e| format!("sign management client request: {e}"))?;
    let management_csr_base64 =
        base64::engine::general_purpose::STANDARD.encode(management_csr.der());
    let request = InitializeRequest {
        schema: INITIALIZE_SCHEMA.into(),
        session_id: gump_types::OperationId::new().to_hyphenated(),
        transcript_digest: transcript_digest(
            &server_parameters,
            &management_csr_base64,
            &options.management_identity_ref,
        )?,
        handoff_binding_digest: handoff.binding_digest.clone(),
        activation_code,
        management_client_csr_der_base64: management_csr_base64,
        management_client_identity_ref: options.management_identity_ref.clone(),
        server_parameters,
    };
    request.validate()?;
    let expected_pin = validate_endpoint_identity(&handoff.endpoint_identity)?;
    let deadline = Instant::now() + options.deadline;
    let mut delay = Duration::from_millis(200);
    let mut management_stored = false;
    loop {
        let response = post_pinned(&handoff.endpoint, expected_pin, &request)?;
        match response.status {
            200 => {
                let result: BootstrapResult = serde_json::from_slice(&response.body)
                    .map_err(|e| format!("invalid bootstrap result: {e}"))?;
                if !management_stored {
                    verify_and_store_management_identity(
                        &result,
                        &management_key,
                        options.management_output_fd,
                        &request.session_id,
                    )?;
                    management_stored = true;
                }
                if !result.management_mtls_verified {
                    continue;
                }
                if result.session_id != request.session_id
                    || result.committed_incarnation != handoff.incarnation
                    || !result.management_mtls_verified
                    || !result.node_admitted
                    || !result.activation_consumed
                    || !result.bootstrap_closed
                {
                    return Err(
                        "bootstrap result does not prove a usable initialized cluster".into(),
                    );
                }
                return Ok(result);
            }
            202 if Instant::now() < deadline => {
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_secs(2));
            }
            202 => return Err("bootstrap initialization deadline exceeded".into()),
            status => {
                let safe = serde_json::from_slice::<serde_json::Value>(&response.body)
                    .ok()
                    .and_then(|value| value.get("safeMessage")?.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "bootstrap endpoint rejected initialization".into());
                return Err(format!("bootstrap HTTP {status}: {safe}"));
            }
        }
    }
}

fn verify_and_store_management_identity(
    result: &BootstrapResult,
    key: &rcgen::KeyPair,
    output_fd: i32,
    session_id: &str,
) -> Result<(), String> {
    let ca = base64::engine::general_purpose::STANDARD
        .decode(&result.management_ca_certificate_der_base64)
        .map_err(|_| "management CA certificate is invalid base64")?;
    let client_certificate = base64::engine::general_purpose::STANDARD
        .decode(&result.management_client_certificate_der_base64)
        .map_err(|_| "management client certificate is invalid base64")?;
    if ca.is_empty()
        || ca.len() > 64 * 1024
        || client_certificate.is_empty()
        || client_certificate.len() > 64 * 1024
    {
        return Err("management certificate material is empty or oversized".into());
    }
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from(ca.clone()))
        .map_err(|e| format!("trust management CA: {e}"))?;
    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(
            vec![CertificateDer::from(client_certificate.clone())],
            rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
                key.serialize_der(),
            )),
        )
        .map_err(|e| format!("build management mTLS client: {e}"))?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let path = format!("/v1/bootstrap/verify/{session_id}");
    let response = management_get(&result.management_endpoint, config, &path)?;
    if response.status != 200 {
        return Err(format!(
            "management mTLS verification returned HTTP {}",
            response.status
        ));
    }
    let status: serde_json::Value = serde_json::from_slice(&response.body)
        .map_err(|e| format!("invalid management verification response: {e}"))?;
    if status.get("status").and_then(serde_json::Value::as_str) != Some("healthy")
        || status
            .get("clusterIdentity")
            .and_then(serde_json::Value::as_str)
            != Some(result.cluster_identity.as_str())
        || status
            .get("nodeIdentity")
            .and_then(serde_json::Value::as_str)
            != Some(result.node_identity.as_str())
    {
        return Err("management mTLS response does not match initialized cluster".into());
    }

    let mut material = serde_json::to_vec(&serde_json::json!({
        "schema":"gump.management-client-material/1",
        "clusterIdentity":result.cluster_identity,
        "nodeIdentity":result.node_identity,
        "endpoint":result.management_endpoint,
        "identityRef":result.management_client_identity_ref,
        "caCertificateDerBase64":result.management_ca_certificate_der_base64,
        "clientCertificateDerBase64":result.management_client_certificate_der_base64,
        "privateKeyPkcs8DerBase64":base64::engine::general_purpose::STANDARD.encode(key.serialize_der())
    }))
    .map_err(|e| format!("encode management client material: {e}"))?;
    let write_result = gump_types::inherited_fd::write_all(output_fd, &material, 256 * 1024)
        .map_err(|e| format!("store management client material: {e}"));
    material.zeroize();
    write_result
}

fn management_get(
    endpoint: &str,
    config: rustls::ClientConfig,
    path: &str,
) -> Result<HttpResponse, String> {
    let url = url::Url::parse(endpoint).map_err(|_| "management endpoint is not a valid URL")?;
    if url.scheme() != "https"
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("management endpoint must be an https origin".into());
    }
    let host = url.host_str().ok_or("management endpoint has no host")?;
    let port = url
        .port_or_known_default()
        .ok_or("management endpoint has no port")?;
    let mut tcp = TcpStream::connect(format_host_port(host, port))
        .map_err(|e| format!("connect management endpoint: {e}"))?;
    tcp.set_read_timeout(Some(Duration::from_secs(10))).ok();
    tcp.set_write_timeout(Some(Duration::from_secs(10))).ok();
    let server_name = ServerName::try_from("localhost")
        .map_err(|_| "construct management TLS name")?
        .to_owned();
    let mut connection = rustls::ClientConnection::new(Arc::new(config), server_name)
        .map_err(|e| format!("create management TLS client: {e}"))?;
    let mut tls = rustls::Stream::new(&mut connection, &mut tcp);
    write!(
        tls,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .map_err(|e| format!("write management verification request: {e}"))?;
    tls.flush()
        .map_err(|e| format!("flush management verification request: {e}"))?;
    read_http_response(&mut tls)
}

fn read_fd_bounded(fd: i32, max: usize, label: &str) -> Result<Vec<u8>, String> {
    if fd < 3 {
        return Err(format!("{label} descriptor must be >= 3"));
    }
    let bytes = gump_types::inherited_fd::read_bounded(fd, max + 1)
        .map_err(|e| format!("read {label} descriptor: {e}"))?;
    if bytes.len() > max {
        return Err(format!("{label} exceeds {max} bytes"));
    }
    Ok(bytes)
}

fn trim_ascii_whitespace(bytes: &mut Vec<u8>) {
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes.pop();
    }
}

struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

fn post_pinned(
    endpoint: &str,
    expected_pin: [u8; 32],
    request: &InitializeRequest,
) -> Result<HttpResponse, String> {
    let url = url::Url::parse(endpoint).map_err(|_| "bootstrap endpoint is not a valid URL")?;
    if url.scheme() != "https"
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "bootstrap endpoint must be an https origin without path, query, or fragment".into(),
        );
    }
    let host = url.host_str().ok_or("bootstrap endpoint has no host")?;
    let port = url
        .port_or_known_default()
        .ok_or("bootstrap endpoint has no port")?;
    let address = format_host_port(host, port);
    let mut tcp =
        TcpStream::connect(&address).map_err(|e| format!("connect bootstrap endpoint: {e}"))?;
    tcp.set_read_timeout(Some(Duration::from_secs(10))).ok();
    tcp.set_write_timeout(Some(Duration::from_secs(10))).ok();

    let verifier = Arc::new(SpkiPinVerifier::new(expected_pin));
    let mut config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let server_name = ServerName::try_from("localhost")
        .map_err(|_| "construct bootstrap TLS name")?
        .to_owned();
    let mut connection = rustls::ClientConnection::new(Arc::new(config), server_name)
        .map_err(|e| format!("create bootstrap TLS client: {e}"))?;
    let mut tls = rustls::Stream::new(&mut connection, &mut tcp);
    let mut body =
        serde_json::to_vec(request).map_err(|e| format!("encode bootstrap request: {e}"))?;
    if body.len() > MAX_INITIALIZE_BYTES {
        body.zeroize();
        return Err("bootstrap initialization request exceeds protocol bound".into());
    }
    write!(
        tls,
        "POST /v1/bootstrap/initialize HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .map_err(|e| format!("write bootstrap request headers: {e}"))?;
    tls.write_all(&body)
        .map_err(|e| format!("write bootstrap request: {e}"))?;
    body.zeroize();
    tls.flush()
        .map_err(|e| format!("flush bootstrap request: {e}"))?;
    read_http_response(&mut tls)
}

fn format_host_port(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn read_http_response(stream: &mut impl Read) -> Result<HttpResponse, String> {
    let mut bytes = Vec::new();
    let header_end = loop {
        if bytes.len() >= 16 * 1024 {
            return Err("bootstrap response headers exceed limit".into());
        }
        let mut chunk = [0_u8; 1024];
        let read = stream
            .read(&mut chunk)
            .map_err(|e| format!("read bootstrap response: {e}"))?;
        if read == 0 {
            return Err("bootstrap response ended before headers".into());
        }
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| "bootstrap response headers are not UTF-8")?;
    let mut lines = headers.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or("bootstrap response has no status")?
        .parse::<u16>()
        .map_err(|_| "bootstrap response status is invalid")?;
    let mut content_length = None;
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .ok_or("malformed bootstrap response header")?;
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err("duplicate bootstrap response Content-Length".into());
            }
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "invalid bootstrap response Content-Length")?,
            );
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err("bootstrap response Transfer-Encoding is unsupported".into());
        }
    }
    let content_length = content_length.ok_or("bootstrap response has no Content-Length")?;
    if content_length > MAX_RESPONSE_BYTES {
        return Err("bootstrap response body exceeds limit".into());
    }
    if bytes.len() - header_end > content_length {
        return Err("bootstrap response contains trailing bytes".into());
    }
    while bytes.len() - header_end < content_length {
        let remaining = content_length - (bytes.len() - header_end);
        let mut chunk = [0_u8; 1024];
        let requested = remaining.min(chunk.len());
        let read = stream
            .read(&mut chunk[..requested])
            .map_err(|e| format!("read bootstrap response body: {e}"))?;
        if read == 0 {
            return Err("bootstrap response body ended early".into());
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(HttpResponse {
        status,
        body: bytes[header_end..].to_vec(),
    })
}

#[derive(Debug)]
struct SpkiPinVerifier {
    expected: [u8; 32],
    algorithms: rustls::crypto::WebPkiSupportedAlgorithms,
}

impl SpkiPinVerifier {
    fn new(expected: [u8; 32]) -> Self {
        Self {
            expected,
            algorithms: rustls::crypto::ring::default_provider().signature_verification_algorithms,
        }
    }
}

impl ServerCertVerifier for SpkiPinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let (_, certificate) =
            x509_parser::certificate::X509Certificate::from_der(end_entity.as_ref())
                .map_err(|_| rustls::Error::General("bootstrap certificate is malformed".into()))?;
        let actual: [u8; 32] = Sha256::digest(certificate.public_key().raw).into();
        if !constant_time_eq(&actual, &self.expected) {
            return Err(rustls::Error::General(
                "bootstrap endpoint SPKI pin mismatch".into(),
            ));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut difference = a.len() ^ b.len();
    for index in 0..a.len().max(b.len()) {
        difference |=
            usize::from(a.get(index).copied().unwrap_or(0) ^ b.get(index).copied().unwrap_or(0));
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_ipv4_and_ipv6_authorities() {
        assert_eq!(format_host_port("127.0.0.1", 7443), "127.0.0.1:7443");
        assert_eq!(format_host_port("::1", 7443), "[::1]:7443");
    }

    #[test]
    fn endpoint_spki_pin_accepts_exact_key_and_rejects_mismatch() {
        let key = rcgen::KeyPair::generate().unwrap();
        let certificate = rcgen::CertificateParams::new(vec!["localhost".into()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        let der = CertificateDer::from(certificate.der().to_vec());
        let (_, parsed) =
            x509_parser::certificate::X509Certificate::from_der(der.as_ref()).unwrap();
        let expected: [u8; 32] = Sha256::digest(parsed.public_key().raw).into();
        let server_name = ServerName::try_from("localhost").unwrap();
        assert!(
            SpkiPinVerifier::new(expected)
                .verify_server_cert(&der, &[], &server_name, &[], UnixTime::now())
                .is_ok()
        );
        assert!(
            SpkiPinVerifier::new([0_u8; 32])
                .verify_server_cert(&der, &[], &server_name, &[], UnixTime::now())
                .is_err()
        );
    }
}
