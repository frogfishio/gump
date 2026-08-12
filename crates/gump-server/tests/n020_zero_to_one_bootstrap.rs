//! Zero-to-one acceptance: activation, SPKI pin, exact claim, runtime start,
//! management mTLS proof, secret-material descriptor output, and closure.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt as _;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use gump_protocol::bootstrap::{
    ActivationBundle, BOOTSTRAP_PROTOCOL, BootstrapHandoff, HANDOFF_SCHEMA,
};
use gump_transport::{NodeRole, TransportIdentity, mint_identity};
use gump_types::{ClusterId, IncarnationId, NodeId};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        encoded.push(DIGITS[(byte >> 4) as usize] as char);
        encoded.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn management_snapshot(endpoint: &str, material: &serde_json::Value) -> serde_json::Value {
    let decode = |field: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(material[field].as_str().unwrap())
            .unwrap()
    };
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from(decode("caCertificateDerBase64")))
        .unwrap();
    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(
            vec![CertificateDer::from(decode("clientCertificateDerBase64"))],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(decode("privateKeyPkcs8DerBase64"))),
        )
        .unwrap();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let address = endpoint.strip_prefix("https://").unwrap();
    let mut tcp = TcpStream::connect(address).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut connection = rustls::ClientConnection::new(
        Arc::new(config),
        ServerName::try_from("localhost").unwrap().to_owned(),
    )
    .unwrap();
    let mut stream = rustls::Stream::new(&mut connection, &mut tcp);
    write!(
        stream,
        "GET /v1/captain/snapshot HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    stream.flush().unwrap();
    let mut response = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => response.extend_from_slice(&chunk[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error) => panic!("read management snapshot: {error}"),
        }
    }
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap()
        + 4;
    assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
    serde_json::from_slice(&response[header_end..]).unwrap()
}

#[test]
fn bootstrap_reaches_real_management_mtls_and_closes() {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let bootstrap_port = free_port();
    let management_port = free_port();
    let cluster_port = free_port();
    let bootstrap_endpoint = format!("https://127.0.0.1:{bootstrap_port}");
    let management_endpoint = format!("https://127.0.0.1:{management_port}");
    let socket = directory.path().join("gump.sock");
    let state_root = directory.path().join("state");
    let binary = env!("CARGO_BIN_EXE_gump");
    let child = Command::new(binary)
        .args([
            "server",
            "--bootstrap",
            "--bootstrap-bind",
            &format!("127.0.0.1:{bootstrap_port}"),
            "--advertise-bootstrap",
            &bootstrap_endpoint,
            "--management-bind",
            &format!("127.0.0.1:{management_port}"),
            "--advertise-management",
            &management_endpoint,
            "--runtime-directory",
            directory.path().to_str().unwrap(),
            "--allow-non-tmpfs-for-test",
            "--memory-object-store",
            "--socket",
            socket.to_str().unwrap(),
            "--state-root",
            state_root.to_str().unwrap(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut server = ChildGuard(child);

    let activation_path = directory.path().join("bootstrap.json");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !activation_path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(25));
    }
    let activation: ActivationBundle =
        serde_json::from_slice(&fs::read(&activation_path).unwrap()).unwrap();
    let mut handoff = BootstrapHandoff {
        schema: HANDOFF_SCHEMA.into(),
        handoff_id: "acceptance-handoff".into(),
        incarnation: activation.incarnation.clone(),
        endpoint: activation.endpoint.clone(),
        bootstrap_protocol: BOOTSTRAP_PROTOCOL.into(),
        build_identity: activation.build_identity.clone(),
        machine_identity: "test/local".into(),
        ssh_trust_mode: "operator-accepted".into(),
        ssh_host_key: "SHA256:test-host-key".into(),
        endpoint_identity: activation.endpoint_identity.clone(),
        expires_at: activation.expires_at.clone(),
        binding_digest: String::new(),
        secret_ref: "secret://test/bootstrap".into(),
    };
    handoff.binding_digest = handoff.computed_binding_digest().unwrap();
    let handoff_path = directory.path().join("handoff.json");
    let secret_path = directory.path().join("activation.secret");
    let parameters_path = directory.path().join("parameters.json");
    let management_path = directory.path().join("management.json");
    fs::write(&handoff_path, serde_json::to_vec(&handoff).unwrap()).unwrap();
    fs::write(&secret_path, activation.activation_code.as_bytes()).unwrap();
    let cluster_id = ClusterId::new();
    let node_id = NodeId::new();
    let (transport, _) = mint_identity(TransportIdentity {
        cluster_id,
        node_id,
        incarnation: IncarnationId::new(),
        roles: vec![
            NodeRole::Memory,
            NodeRole::Agent,
            NodeRole::Controller,
            NodeRole::Ingress,
        ],
    })
    .unwrap();
    let parameters = serde_json::json!({
        "cluster_id":cluster_id.to_hyphenated(),
        "recovery_secret_hex":"11".repeat(32),
        "cluster_transport":{
            "bind":format!("127.0.0.1:{cluster_port}"),
            "advertise":format!("127.0.0.1:{cluster_port}"),
            "certificate_der_hex":hex(transport.certificate_der()),
            "private_key_pkcs8_der_hex":hex(transport.private_key_der()),
            "ca_certificate_der_hex":hex(transport.ca_certificate_der()),
            "join_token":null,
            "allowed_join_tokens":[]
        }
    });
    fs::write(&parameters_path, serde_json::to_vec(&parameters).unwrap()).unwrap();

    let output = Command::new("/bin/bash")
        .arg("-c")
        .arg(
            "exec 3<\"$1\"; exec 4<\"$2\"; exec 5<\"$3\"; exec 6>\"$4\"; exec \"$5\" bootstrap initialize --handoff-fd 3 --activation-fd 4 --initialization-fd 5 --management-output-fd 6 --management-identity-ref secret://test/management --deadline-ms 10000",
        )
        .arg("gump-bootstrap-test")
        .arg(&handoff_path)
        .arg(&secret_path)
        .arg(&parameters_path)
        .arg(&management_path)
        .arg(binary)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "bootstrap CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "committed");
    assert_eq!(result["managementMtlsVerified"], true);
    assert_eq!(result["nodeAdmitted"], true);
    assert_eq!(result["nodeIdentity"], node_id.to_hyphenated());
    assert_eq!(result["activationConsumed"], true);
    assert_eq!(result["bootstrapClosed"], true);
    assert!(!activation_path.exists());
    let management: serde_json::Value =
        serde_json::from_slice(&fs::read(&management_path).unwrap()).unwrap();
    assert_eq!(management["schema"], "gump.management-client-material/1");
    assert!(
        management["privateKeyPkcs8DerBase64"]
            .as_str()
            .unwrap()
            .len()
            > 40
    );

    let snapshot = management_snapshot(&management_endpoint, &management);
    assert_eq!(snapshot["schema"], "gump.captain-snapshot/1");
    assert_eq!(snapshot["protocol"], "gump.captain-control/1");
    assert_eq!(snapshot["clusterIdentity"], cluster_id.to_hyphenated());
    assert_eq!(snapshot["nodeIdentity"], node_id.to_hyphenated());
    assert_eq!(snapshot["consistency"], "linearizable");
    assert_eq!(snapshot["cluster"]["voterCount"], 1);
    assert_eq!(snapshot["cluster"]["custody"], "unsealed");
    assert_eq!(snapshot["workloads"], serde_json::json!([]));

    server.0.kill().unwrap();
    let _ = server.0.wait();
}
