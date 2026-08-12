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
use crate::serve::LocalDaemon;
use gump_protocol::captain_control::{
    CAPTAIN_CONTROL_PROTOCOL, CAPTAIN_ERROR_SCHEMA, CAPTAIN_SNAPSHOT_SCHEMA,
    CaptainClusterSnapshotV1, CaptainControlErrorV1, CaptainLocalExecutionSnapshotV1,
    CaptainSnapshotLimitsV1, CaptainSnapshotV1, CaptainWorkloadSnapshotV1, MAX_SNAPSHOT_BYTES,
    MAX_SNAPSHOT_WORKLOADS,
};

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
    daemon: Arc<LocalDaemon>,
}

impl ManagementBootstrapEndpoint {
    pub fn bind(
        bind: &str,
        authority: &ManagementAuthority,
        session_id: String,
        result: InitializationResult,
        cluster_identity: String,
        node_identity: String,
        daemon: Arc<LocalDaemon>,
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
            daemon,
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
            if let Err(error) = self.handle(&mut stream) {
                let _ = write_response(&mut stream, error.status, &error.body);
            }
        }
    }

    fn handle(&self, stream: &mut (impl Read + Write)) -> Result<(), ManagementRequestError> {
        let mut bytes = Vec::with_capacity(1024);
        loop {
            if bytes.len() >= 8 * 1024 {
                return Err(ManagementRequestError::invalid(
                    "management request headers exceed limit",
                ));
            }
            let mut chunk = [0_u8; 512];
            let read = stream
                .read(&mut chunk)
                .map_err(|e| ManagementRequestError::invalid(format!("read request: {e}")))?;
            if read == 0 {
                return Err(ManagementRequestError::invalid(
                    "management request ended early",
                ));
            }
            bytes.extend_from_slice(&chunk[..read]);
            if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let request = std::str::from_utf8(&bytes)
            .map_err(|_| ManagementRequestError::invalid("management request is not UTF-8"))?;
        let line = request.split("\r\n").next().unwrap_or_default();
        let verification_path = format!("GET /v1/bootstrap/verify/{} HTTP/1.1", self.session_id);
        if line == verification_path {
            let (lock, ready) = &*self.result;
            let mut slot = lock
                .lock()
                .map_err(|_| ManagementRequestError::unavailable("bootstrap result poisoned"))?;
            let result = slot
                .as_mut()
                .and_then(|result| result.as_mut().ok())
                .ok_or_else(|| {
                    ManagementRequestError::unavailable("bootstrap result is unavailable")
                })?;
            if result.bootstrap_closed {
                return Err(ManagementRequestError::not_found(
                    "bootstrap verification route is closed",
                ));
            }
            result.management_mtls_verified = true;
            ready.notify_all();
            return write_response(
                stream,
                200,
                &serde_json::json!({
                    "schema":"gump.management-status/1",
                    "clusterIdentity":self.cluster_identity,
                    "nodeIdentity":self.node_identity,
                    "status":"healthy"
                }),
            )
            .map_err(ManagementRequestError::io);
        }
        if line == "GET /v1/status HTTP/1.1" {
            return write_response(
                stream,
                200,
                &serde_json::json!({
                    "schema":"gump.management-status/1",
                    "clusterIdentity":self.cluster_identity,
                    "nodeIdentity":self.node_identity,
                    "status":"healthy"
                }),
            )
            .map_err(ManagementRequestError::io);
        }
        if line == "GET /v1/captain/snapshot HTTP/1.1" {
            let snapshot = self.captain_snapshot()?;
            let bytes = serde_json::to_vec(&snapshot).map_err(|error| {
                ManagementRequestError::internal(format!("encode snapshot: {error}"))
            })?;
            if bytes.len() > MAX_SNAPSHOT_BYTES {
                return Err(ManagementRequestError::too_large(
                    "snapshot exceeds the response-byte ceiling",
                ));
            }
            return write_response_bytes(stream, 200, &bytes).map_err(ManagementRequestError::io);
        }
        Err(ManagementRequestError::not_found(
            "management route not found",
        ))
    }

    fn captain_snapshot(&self) -> Result<CaptainSnapshotV1, ManagementRequestError> {
        build_captain_snapshot(&self.daemon, &self.cluster_identity, &self.node_identity)
    }
}

fn build_captain_snapshot(
    daemon: &LocalDaemon,
    cluster_identity: &str,
    node_identity: &str,
) -> Result<CaptainSnapshotV1, ManagementRequestError> {
    let memory = daemon.memory_cluster.as_ref().ok_or_else(|| {
        ManagementRequestError::unavailable("this node has no memory/control facet")
    })?;
    let cut = memory.control_snapshot().map_err(|error| {
        ManagementRequestError::unavailable(format!("linearizable snapshot unavailable: {error}"))
    })?;
    if cut.desired.len() > MAX_SNAPSHOT_WORKLOADS {
        return Err(ManagementRequestError::too_large(
            "workload count exceeds the snapshot ceiling",
        ));
    }
    let workloads = cut
        .desired
        .into_iter()
        .map(|entry| CaptainWorkloadSnapshotV1 {
            namespace: entry.namespace,
            app: entry.app,
            generation: entry.generation,
            capsule_digest: hex(&entry.content_digest),
        })
        .collect();
    let custody = daemon
        .custody
        .as_ref()
        .and_then(|custody| custody.lock().ok().map(|guard| guard.is_sealed()))
        .map_or(
            "unavailable",
            |sealed| {
                if sealed { "sealed" } else { "unsealed" }
            },
        )
        .to_string();
    let local_execution = daemon
        .execution
        .as_ref()
        .map(|execution| {
            let status = execution
                .lock()
                .map_err(|_| {
                    ManagementRequestError::unavailable("local execution status is unavailable")
                })?
                .status();
            Ok(CaptainLocalExecutionSnapshotV1 {
                scope: "node_local".into(),
                desired: usize_to_u64(status.desired)?,
                placements: usize_to_u64(status.placements)?,
                completed: usize_to_u64(status.completed)?,
                ready: usize_to_u64(status.ready)?,
                hiccup_presence: usize_to_u64(status.hiccup_presence)?,
                degraded: status.last_error.is_some(),
                s3_head_requests: status.s3_head_requests,
                s3_full_get_requests: status.s3_full_get_requests,
                s3_ranged_get_requests: status.s3_ranged_get_requests,
                s3_bytes_read: status.s3_bytes_read,
            })
        })
        .transpose()?;
    let voter_count = u32::try_from(cut.voters.len()).map_err(|_| {
        ManagementRequestError::too_large("voter count exceeds the wire representation")
    })?;
    Ok(CaptainSnapshotV1 {
        schema: CAPTAIN_SNAPSHOT_SCHEMA.into(),
        protocol: CAPTAIN_CONTROL_PROTOCOL.into(),
        cluster_identity: cluster_identity.into(),
        node_identity: node_identity.into(),
        consistency: "linearizable".into(),
        revision: cut.revision,
        cluster: CaptainClusterSnapshotV1 {
            raft_node_id: cut.status.node_id,
            current_leader: cut.status.current_leader,
            voters: cut.voters,
            voter_count,
            controller_epoch: cut.status.controller_epoch,
            controller_holder: cut.status.controller_holder,
            durable_cluster_state: cut.status.durable_cluster_state,
            custody,
        },
        workloads,
        local_execution,
        limits: CaptainSnapshotLimitsV1 {
            max_workloads: MAX_SNAPSHOT_WORKLOADS as u32,
            max_response_bytes: MAX_SNAPSHOT_BYTES as u32,
        },
    })
}

#[derive(Debug)]
struct ManagementRequestError {
    status: u16,
    body: CaptainControlErrorV1,
}

impl ManagementRequestError {
    fn new(status: u16, code: &str, retryable: bool, detail: impl AsRef<str>) -> Self {
        Self {
            status,
            body: CaptainControlErrorV1 {
                schema: CAPTAIN_ERROR_SCHEMA.into(),
                code: code.into(),
                retryable,
                detail: bounded_detail(detail.as_ref()),
            },
        }
    }

    fn invalid(detail: impl AsRef<str>) -> Self {
        Self::new(400, "INVALID_REQUEST", false, detail)
    }

    fn not_found(detail: impl AsRef<str>) -> Self {
        Self::new(404, "NOT_FOUND", false, detail)
    }

    fn unavailable(detail: impl AsRef<str>) -> Self {
        Self::new(503, "SNAPSHOT_UNAVAILABLE", true, detail)
    }

    fn too_large(detail: impl AsRef<str>) -> Self {
        Self::new(413, "SNAPSHOT_LIMIT_EXCEEDED", false, detail)
    }

    fn internal(detail: impl AsRef<str>) -> Self {
        Self::new(500, "INTERNAL", true, detail)
    }

    fn io(detail: impl AsRef<str>) -> Self {
        Self::new(500, "IO", true, detail)
    }
}

fn usize_to_u64(value: usize) -> Result<u64, ManagementRequestError> {
    u64::try_from(value)
        .map_err(|_| ManagementRequestError::too_large("counter exceeds wire representation"))
}

fn bounded_detail(detail: &str) -> String {
    detail
        .chars()
        .filter(|character| !character.is_control())
        .take(1024)
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn write_response(
    stream: &mut impl Write,
    status: u16,
    body: &impl serde::Serialize,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(body).map_err(|e| e.to_string())?;
    write_response_bytes(stream, status, &bytes)
}

fn write_response_bytes(stream: &mut impl Write, status: u16, bytes: &[u8]) -> Result<(), String> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        413 => "Content Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        bytes.len()
    )
    .map_err(|e| e.to_string())?;
    stream.write_all(bytes).map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gump_memory::{MemoryCluster, RaftCommand, RaftResponse};

    use crate::peer::PeerAllowlist;

    #[test]
    fn captain_snapshot_excludes_opaque_desired_payload() {
        let cluster = Arc::new(MemoryCluster::bootstrap_one_voter(1, 9).expect("cluster"));
        let response = cluster
            .client_write(RaftCommand::PutDesired {
                namespace: "default".into(),
                app: "database".into(),
                expected_generation: 0,
                payload: b"protected-or-opaque-configuration".to_vec(),
                content_digest: [0xab; 32],
            })
            .expect("desired write");
        assert!(matches!(response, RaftResponse::Applied(_)));

        let mut daemon = LocalDaemon::new(PeerAllowlist::same_uid(0));
        daemon.memory_cluster = Some(cluster);
        let snapshot = build_captain_snapshot(&daemon, "cluster", "node").expect("snapshot");
        assert_eq!(snapshot.schema, CAPTAIN_SNAPSHOT_SCHEMA);
        assert_eq!(snapshot.consistency, "linearizable");
        assert_eq!(snapshot.cluster.controller_holder, Some(9));
        assert_eq!(snapshot.workloads.len(), 1);
        assert_eq!(snapshot.workloads[0].capsule_digest, "ab".repeat(32));
        let json = serde_json::to_string(&snapshot).expect("JSON");
        assert!(!json.contains("protected-or-opaque-configuration"));
        assert!(json.len() <= MAX_SNAPSHOT_BYTES);
    }

    #[test]
    fn control_error_detail_is_bounded_and_strips_controls() {
        let detail = format!("{}\nsecret", "x".repeat(2_000));
        let error = ManagementRequestError::invalid(detail);
        assert_eq!(error.body.detail.len(), 1024);
        assert!(!error.body.detail.contains('\n'));
    }
}
