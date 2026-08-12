//! Secure activation material and claim state for zero-to-one bootstrap.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::Duration as StdDuration;

use base64::Engine as _;
use gump_protocol::bootstrap::{
    ACTIVATION_SCHEMA, ActivationBundle, BOOTSTRAP_PROTOCOL, BootstrapResult, InitializeRequest,
    MAX_ACTIVATION_BUNDLE_BYTES, MAX_INITIALIZE_BYTES, MAX_RESPONSE_BYTES,
};
use gump_types::{IncarnationId, Secret};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
use rustls::ServerConfig;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest as _, Sha256};
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};
use x509_parser::prelude::FromDer as _;
use zeroize::Zeroize as _;

pub const ACTIVATION_FILE_NAME: &str = "bootstrap.json";
pub const DEFAULT_ACTIVATION_TTL_SECS: i64 = 10 * 60;
const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const CONNECTION_TIMEOUT: StdDuration = StdDuration::from_secs(10);

pub struct BootstrapIdentity {
    tls: Arc<ServerConfig>,
    endpoint_identity: String,
}

impl BootstrapIdentity {
    pub fn generate() -> Result<Self, String> {
        let key = KeyPair::generate().map_err(|e| format!("generate bootstrap TLS key: {e}"))?;
        let mut params = CertificateParams::new(vec!["localhost".into()])
            .map_err(|e| format!("bootstrap certificate parameters: {e}"))?;
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, "gump-bootstrap");
        params.distinguished_name = name;
        let certificate = params
            .self_signed(&key)
            .map_err(|e| format!("generate bootstrap certificate: {e}"))?;
        let certificate_der = CertificateDer::from(certificate.der().to_vec());
        let (_, parsed) =
            x509_parser::certificate::X509Certificate::from_der(certificate_der.as_ref())
                .map_err(|_| "parse generated bootstrap certificate".to_string())?;
        let digest = Sha256::digest(parsed.public_key().raw);
        let endpoint_identity = format!(
            "SHA256:{}",
            base64::engine::general_purpose::STANDARD.encode(digest)
        );
        let private_key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
        let mut tls = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certificate_der], private_key)
            .map_err(|e| format!("build bootstrap TLS configuration: {e}"))?;
        tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Self {
            tls: Arc::new(tls),
            endpoint_identity,
        })
    }

    pub fn tls_config(&self) -> Arc<ServerConfig> {
        Arc::clone(&self.tls)
    }

    pub fn endpoint_identity(&self) -> &str {
        &self.endpoint_identity
    }
}

pub struct ActivationLease {
    bundle: ActivationBundle,
    activation_code: Secret<String>,
    path: PathBuf,
    claimed: bool,
}

impl ActivationLease {
    pub fn create(
        runtime_directory: &Path,
        endpoint: String,
        endpoint_identity: String,
        now: OffsetDateTime,
        require_tmpfs: bool,
    ) -> Result<Self, String> {
        validate_runtime_directory(runtime_directory, require_tmpfs)?;
        let path = runtime_directory.join(ACTIVATION_FILE_NAME);
        reject_existing_path(&path)?;

        let mut random = [0_u8; 32];
        getrandom::fill(&mut random).map_err(|e| format!("generate activation secret: {e}"))?;
        let activation_code = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random);
        random.zeroize();
        let expires_at = (now + Duration::seconds(DEFAULT_ACTIVATION_TTL_SECS))
            .format(&Rfc3339)
            .map_err(|e| format!("format activation expiry: {e}"))?;
        let bundle = ActivationBundle {
            schema: ACTIVATION_SCHEMA.into(),
            incarnation: IncarnationId::new().to_hyphenated(),
            endpoint,
            bootstrap_protocol: BOOTSTRAP_PROTOCOL.into(),
            build_identity: gump_types::product::version_string(),
            endpoint_identity,
            activation_code: activation_code.clone(),
            expires_at,
        };
        let mut encoded =
            serde_json::to_vec(&bundle).map_err(|e| format!("encode activation bundle: {e}"))?;
        if encoded.len() > MAX_ACTIVATION_BUNDLE_BYTES {
            encoded.zeroize();
            return Err("activation bundle exceeds protocol bound".into());
        }
        create_atomic_no_replace(runtime_directory, &path, &encoded)?;
        encoded.zeroize();
        Ok(Self {
            bundle,
            activation_code: Secret::new(activation_code),
            path,
            claimed: false,
        })
    }

    pub fn public_bundle(&self) -> &ActivationBundle {
        &self.bundle
    }

    pub fn is_expired(&self, now: OffsetDateTime) -> bool {
        let Ok(expiry) = OffsetDateTime::parse(&self.bundle.expires_at, &Rfc3339) else {
            return true;
        };
        expiry <= now
    }

    fn authenticate(&self, supplied: &str) -> bool {
        constant_time_eq(
            self.activation_code.expose().as_bytes(),
            supplied.as_bytes(),
        )
    }

    fn remove_file(&mut self) -> Result<(), String> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("remove activation bundle: {error}")),
        }
    }

    fn destroy_secret(&mut self) {
        self.activation_code.expose_mut().zeroize();
        self.bundle.activation_code.zeroize();
    }
}

impl Drop for ActivationLease {
    fn drop(&mut self) {
        self.destroy_secret();
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClaimOutcome {
    Claimed,
    Resumed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Claim {
    session_id: String,
    transcript_digest: String,
    handoff_binding_digest: String,
}

pub struct BootstrapClaimState {
    lease: ActivationLease,
    claim: Option<Claim>,
    consumed: bool,
    expired: bool,
}

/// Result shared between the network bootstrap thread and runtime initializer.
pub type InitializationResult = Arc<(Mutex<Option<Result<BootstrapResult, String>>>, Condvar)>;

pub fn empty_initialization_result() -> InitializationResult {
    Arc::new((Mutex::new(None), Condvar::new()))
}

/// Restricted HTTPS bootstrap endpoint. It has exactly one public liveness
/// route and one authenticated initialization route.
pub struct BootstrapEndpoint {
    listener: TcpListener,
    identity: BootstrapIdentity,
    state: Arc<Mutex<BootstrapClaimState>>,
}

impl BootstrapEndpoint {
    pub fn bind(
        bind: &str,
        advertised_endpoint: String,
        runtime_directory: &Path,
        now: OffsetDateTime,
        require_tmpfs: bool,
    ) -> Result<Self, String> {
        let identity = BootstrapIdentity::generate()?;
        let listener =
            TcpListener::bind(bind).map_err(|e| format!("bind bootstrap endpoint: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("configure bootstrap endpoint: {e}"))?;
        let lease = ActivationLease::create(
            runtime_directory,
            advertised_endpoint,
            identity.endpoint_identity().into(),
            now,
            require_tmpfs,
        )?;
        Ok(Self {
            listener,
            identity,
            state: Arc::new(Mutex::new(BootstrapClaimState::new(lease))),
        })
    }

    pub fn local_address(&self) -> Result<std::net::SocketAddr, String> {
        self.listener
            .local_addr()
            .map_err(|e| format!("read bootstrap address: {e}"))
    }

    pub fn activation(&self) -> Result<ActivationBundle, String> {
        let state = self.state.lock().map_err(|_| "bootstrap state poisoned")?;
        let bundle = state.activation();
        Ok(ActivationBundle {
            schema: bundle.schema.clone(),
            incarnation: bundle.incarnation.clone(),
            endpoint: bundle.endpoint.clone(),
            bootstrap_protocol: bundle.bootstrap_protocol.clone(),
            build_identity: bundle.build_identity.clone(),
            endpoint_identity: bundle.endpoint_identity.clone(),
            activation_code: bundle.activation_code.clone(),
            expires_at: bundle.expires_at.clone(),
        })
    }

    /// Serve until one committed result has been returned to its claiming
    /// session. The first successful claim is delivered exactly once to the
    /// runtime initializer through `claimed`.
    pub fn serve(
        self,
        claimed: mpsc::SyncSender<InitializeRequest>,
        result: InitializationResult,
    ) -> Result<(), String> {
        loop {
            let (mut stream, _) = match self.listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    {
                        let mut state =
                            self.state.lock().map_err(|_| "bootstrap state poisoned")?;
                        if state.expire_if_needed(OffsetDateTime::now_utc())? {
                            return Err("bootstrap activation expired before initialization".into());
                        }
                    }
                    std::thread::sleep(StdDuration::from_millis(100));
                    continue;
                }
                Err(error) => return Err(format!("accept bootstrap connection: {error}")),
            };
            stream.set_nonblocking(false).ok();
            stream.set_read_timeout(Some(CONNECTION_TIMEOUT)).ok();
            stream.set_write_timeout(Some(CONNECTION_TIMEOUT)).ok();
            let mut connection = rustls::ServerConnection::new(self.identity.tls_config())
                .map_err(|e| format!("create bootstrap TLS connection: {e}"))?;
            let mut tls = rustls::Stream::new(&mut connection, &mut stream);
            match handle_bootstrap_http(&mut tls, &self.state, &claimed, &result) {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(_) => {
                    let _ = write_json_response(
                        &mut tls,
                        400,
                        "Bad Request",
                        &serde_json::json!({
                            "schema":"gump.bootstrap-error/1",
                            "code":"INVALID_REQUEST",
                            "safeMessage":"bootstrap request rejected"
                        }),
                    );
                }
            }
        }
    }
}

fn handle_bootstrap_http(
    stream: &mut impl ReadWrite,
    state: &Arc<Mutex<BootstrapClaimState>>,
    claimed: &mpsc::SyncSender<InitializeRequest>,
    result: &InitializationResult,
) -> Result<bool, String> {
    let mut request = read_http_request(stream)?;
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/health") => {
            let state = state.lock().map_err(|_| "bootstrap state poisoned")?;
            let body = serde_json::json!({
                "schema": "gump.bootstrap-status/1",
                "status": if state.is_consumed() { "closed" } else { "awaiting_activation" },
                "incarnation": state.activation().incarnation,
                "bootstrapProtocol": BOOTSTRAP_PROTOCOL,
            });
            write_json_response(stream, 200, "OK", &body)?;
            Ok(false)
        }
        ("POST", "/v1/bootstrap/initialize") => {
            let parsed = serde_json::from_slice(&request.body);
            request.body.zeroize();
            let mut initialize: InitializeRequest =
                parsed.map_err(|_| "invalid bounded bootstrap request".to_string())?;
            let transcript_digest = initialize.transcript_digest.clone();
            let outcome = {
                let mut state = state.lock().map_err(|_| "bootstrap state poisoned")?;
                state.claim(&initialize, OffsetDateTime::now_utc())?
            };
            if outcome == ClaimOutcome::Claimed {
                claimed
                    .send(initialize)
                    .map_err(|_| "runtime initializer is unavailable".to_string())?;
            } else {
                // A resumed request contains secret material which is no longer
                // needed after authentication. Drop it before waiting.
                initialize.activation_code.zeroize();
                initialize.server_parameters = serde_json::Value::Null;
            }

            let (lock, ready) = &**result;
            let guard = lock.lock().map_err(|_| "bootstrap result poisoned")?;
            let (guard, _) = ready
                .wait_timeout(guard, StdDuration::from_secs(2))
                .map_err(|_| "bootstrap result poisoned")?;
            let mut guard = guard;
            match guard.as_mut() {
                None => {
                    write_json_response(
                        stream,
                        202,
                        "Accepted",
                        &serde_json::json!({
                            "schema":"gump.bootstrap-pending/1",
                            "status":"claimed",
                            "retryAfterMs":500
                        }),
                    )?;
                    Ok(false)
                }
                Some(Ok(committed)) => {
                    if !committed.management_mtls_verified {
                        write_json_response(stream, 200, "OK", &committed)?;
                        return Ok(false);
                    }
                    {
                        let mut state = state.lock().map_err(|_| "bootstrap state poisoned")?;
                        state.commit(&committed.session_id, &transcript_digest)?;
                    }
                    committed.activation_consumed = true;
                    committed.bootstrap_closed = true;
                    write_json_response(stream, 200, "OK", &committed)?;
                    Ok(true)
                }
                Some(Err(message)) => {
                    write_json_response(
                        stream,
                        500,
                        "Internal Server Error",
                        &serde_json::json!({
                            "schema":"gump.bootstrap-error/1",
                            "code":"INITIALIZATION_FAILED",
                            "safeMessage":bounded_safe_message(message)
                        }),
                    )?;
                    Ok(true)
                }
            }
        }
        _ => {
            write_json_response(
                stream,
                404,
                "Not Found",
                &serde_json::json!({
                    "schema":"gump.bootstrap-error/1",
                    "code":"NOT_FOUND",
                    "safeMessage":"bootstrap route not found"
                }),
            )?;
            Ok(false)
        }
    }
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

fn read_http_request(stream: &mut impl Read) -> Result<HttpRequest, String> {
    let mut bytes = Vec::with_capacity(2048);
    let header_end = loop {
        if bytes.len() >= MAX_HTTP_HEADER_BYTES {
            return Err("bootstrap HTTP headers exceed limit".into());
        }
        let mut chunk = [0_u8; 1024];
        let read = stream
            .read(&mut chunk)
            .map_err(|error| match error.kind() {
                ErrorKind::TimedOut | ErrorKind::WouldBlock => "bootstrap request timed out".into(),
                _ => format!("read bootstrap request: {error}"),
            })?;
        if read == 0 {
            return Err("bootstrap request ended before headers".into());
        }
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| "bootstrap HTTP headers are not UTF-8")?;
    let mut lines = headers.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split_whitespace();
    let method = request_line
        .next()
        .ok_or("missing HTTP method")?
        .to_string();
    let path = request_line.next().ok_or("missing HTTP path")?.to_string();
    if request_line.next() != Some("HTTP/1.1") || request_line.next().is_some() {
        return Err("bootstrap requires HTTP/1.1".into());
    }
    let mut content_length = None;
    let mut content_type_ok = method == "GET";
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or("malformed HTTP header")?;
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err("duplicate Content-Length".into());
            }
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "invalid Content-Length")?,
            );
        } else if name.eq_ignore_ascii_case("content-type") {
            content_type_ok = value.trim().eq_ignore_ascii_case("application/json");
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err("Transfer-Encoding is not supported".into());
        }
    }
    if !content_type_ok {
        return Err("bootstrap POST requires application/json".into());
    }
    let content_length = content_length.unwrap_or(0);
    if content_length > MAX_INITIALIZE_BYTES {
        return Err("bootstrap request body exceeds limit".into());
    }
    let already = bytes.len() - header_end;
    if already > content_length {
        return Err("bootstrap request contains trailing bytes".into());
    }
    while bytes.len() - header_end < content_length {
        let remaining = content_length - (bytes.len() - header_end);
        let mut chunk = [0_u8; 1024];
        let requested = remaining.min(chunk.len());
        let read = stream
            .read(&mut chunk[..requested])
            .map_err(|e| format!("read bootstrap body: {e}"))?;
        if read == 0 {
            return Err("bootstrap request body ended early".into());
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    let body = bytes[header_end..].to_vec();
    bytes.zeroize();
    Ok(HttpRequest { method, path, body })
}

fn write_json_response(
    stream: &mut impl Write,
    status: u16,
    reason: &str,
    value: &impl serde::Serialize,
) -> Result<(), String> {
    let body = serde_json::to_vec(value).map_err(|e| format!("encode bootstrap response: {e}"))?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err("bootstrap response exceeds limit".into());
    }
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    )
    .map_err(|e| format!("write bootstrap response headers: {e}"))?;
    stream
        .write_all(&body)
        .map_err(|e| format!("write bootstrap response: {e}"))?;
    stream
        .flush()
        .map_err(|e| format!("flush bootstrap response: {e}"))
}

fn bounded_safe_message(message: &str) -> String {
    message
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect()
}

impl BootstrapClaimState {
    pub fn new(lease: ActivationLease) -> Self {
        Self {
            lease,
            claim: None,
            consumed: false,
            expired: false,
        }
    }

    pub fn activation(&self) -> &ActivationBundle {
        self.lease.public_bundle()
    }

    pub fn claim(
        &mut self,
        request: &InitializeRequest,
        now: OffsetDateTime,
    ) -> Result<ClaimOutcome, String> {
        request.validate()?;
        if self.consumed {
            return Err("bootstrap activation has been consumed".into());
        }
        if self.expired {
            return Err("bootstrap activation has expired".into());
        }
        if self.lease.is_expired(now) {
            self.lease.remove_file()?;
            self.lease.destroy_secret();
            self.expired = true;
            return Err("bootstrap activation has expired".into());
        }
        if !self.lease.authenticate(&request.activation_code) {
            return Err("bootstrap authentication failed".into());
        }
        let incoming = Claim {
            session_id: request.session_id.clone(),
            transcript_digest: request.transcript_digest.clone(),
            handoff_binding_digest: request.handoff_binding_digest.clone(),
        };
        match &self.claim {
            None => {
                self.claim = Some(incoming);
                self.lease.claimed = true;
                self.lease.remove_file()?;
                Ok(ClaimOutcome::Claimed)
            }
            Some(existing) if existing == &incoming => Ok(ClaimOutcome::Resumed),
            Some(existing) if existing.session_id == incoming.session_id => {
                Err("claimed session presented a changed transcript".into())
            }
            Some(_) => Err("bootstrap activation is claimed by another session".into()),
        }
    }

    pub fn commit(&mut self, session_id: &str, transcript_digest: &str) -> Result<(), String> {
        if self.consumed {
            return Err("bootstrap activation has been consumed".into());
        }
        if self.expired {
            return Err("bootstrap activation has expired".into());
        }
        let claim = self.claim.as_ref().ok_or("bootstrap is not claimed")?;
        if claim.session_id != session_id || claim.transcript_digest != transcript_digest {
            return Err("commit does not match the claimed session and transcript".into());
        }
        self.consumed = true;
        self.lease.destroy_secret();
        Ok(())
    }

    pub fn is_consumed(&self) -> bool {
        self.consumed
    }

    pub fn expire_if_needed(&mut self, now: OffsetDateTime) -> Result<bool, String> {
        if self.consumed || self.expired || !self.lease.is_expired(now) {
            return Ok(false);
        }
        self.lease.remove_file()?;
        self.lease.destroy_secret();
        self.expired = true;
        Ok(true)
    }
}

fn validate_runtime_directory(path: &Path, require_tmpfs: bool) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| format!("inspect runtime directory {}: {e}", path.display()))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err("bootstrap runtime path must be a real directory".into());
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err("bootstrap runtime directory is not owned by the service user".into());
    }
    if metadata.permissions().mode() & 0o777 != 0o700 {
        return Err("bootstrap runtime directory mode must be 0700".into());
    }
    if require_tmpfs && !is_tmpfs(path)? {
        return Err("bootstrap runtime directory is not memory-backed tmpfs".into());
    }
    Ok(())
}

fn reject_existing_path(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err("bootstrap activation path already exists".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("inspect bootstrap activation path: {error}")),
    }
}

fn create_atomic_no_replace(
    directory: &Path,
    final_path: &Path,
    bytes: &[u8],
) -> Result<(), String> {
    let temporary = directory.join(format!(".bootstrap.{}.tmp", IncarnationId::new()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|e| format!("create activation temporary file: {e}"))?;
        file.write_all(bytes)
            .map_err(|e| format!("write activation bundle: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("sync activation bundle: {e}"))?;
        fs::hard_link(&temporary, final_path)
            .map_err(|e| format!("publish activation bundle without replacement: {e}"))?;
        fs::remove_file(&temporary)
            .map_err(|e| format!("remove activation temporary link: {e}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(target_os = "linux")]
fn is_tmpfs(path: &Path) -> Result<bool, String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| "runtime path contains NUL".to_string())?;
    let mut stats = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `path` is a valid NUL-terminated path and `stats` is writable.
    let result = unsafe { libc::statfs(path.as_ptr(), stats.as_mut_ptr()) };
    if result != 0 {
        return Err(format!(
            "statfs runtime directory: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: successful statfs initialized the structure.
    let stats = unsafe { stats.assume_init() };
    const TMPFS_MAGIC: libc::c_long = 0x0102_1994;
    Ok(stats.f_type == TMPFS_MAGIC)
}

#[cfg(not(target_os = "linux"))]
fn is_tmpfs(_path: &Path) -> Result<bool, String> {
    Ok(false)
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
    use gump_protocol::bootstrap::{INITIALIZE_SCHEMA, transcript_digest};
    use std::os::unix::fs::symlink;

    fn runtime_directory() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        directory
    }

    fn state(directory: &Path, now: OffsetDateTime) -> BootstrapClaimState {
        let identity = BootstrapIdentity::generate().unwrap();
        let lease = ActivationLease::create(
            directory,
            "https://127.0.0.1:7443".into(),
            identity.endpoint_identity().into(),
            now,
            false,
        )
        .unwrap();
        BootstrapClaimState::new(lease)
    }

    fn request(state: &BootstrapClaimState, session: &str) -> InitializeRequest {
        let parameters = serde_json::json!({"cluster_id": null});
        InitializeRequest {
            schema: INITIALIZE_SCHEMA.into(),
            session_id: session.into(),
            transcript_digest: transcript_digest(&parameters, "Y3Ny", "secret://identity").unwrap(),
            handoff_binding_digest: format!("sha256:{}", "a".repeat(64)),
            activation_code: state.activation().activation_code.clone(),
            management_client_csr_der_base64: "Y3Ny".into(),
            management_client_identity_ref: "secret://identity".into(),
            server_parameters: parameters,
        }
    }

    #[test]
    fn activation_file_is_owner_only_and_removed_on_drop() {
        let directory = runtime_directory();
        let now = OffsetDateTime::UNIX_EPOCH;
        let state = state(directory.path(), now);
        let path = directory.path().join(ACTIVATION_FILE_NAME);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let on_disk = fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains(&state.activation().activation_code));
        drop(state);
        assert!(!path.exists());
    }

    #[test]
    fn claim_is_idempotent_only_for_same_session_and_transcript() {
        let directory = runtime_directory();
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut state = state(directory.path(), now);
        let first = request(&state, "session-a");
        assert_eq!(state.claim(&first, now).unwrap(), ClaimOutcome::Claimed);
        assert!(!directory.path().join(ACTIVATION_FILE_NAME).exists());
        assert_eq!(state.claim(&first, now).unwrap(), ClaimOutcome::Resumed);

        let another = request(&state, "session-b");
        assert_eq!(
            state.claim(&another, now).unwrap_err(),
            "bootstrap activation is claimed by another session"
        );
        state
            .commit(&first.session_id, &first.transcript_digest)
            .unwrap();
        assert!(state.is_consumed());
        assert_eq!(
            state.claim(&first, now).unwrap_err(),
            "bootstrap activation has been consumed"
        );
    }

    #[test]
    fn changed_transcript_and_wrong_secret_fail_closed() {
        let directory = runtime_directory();
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut state = state(directory.path(), now);
        let first = request(&state, "session-a");
        state.claim(&first, now).unwrap();
        let mut changed = request(&state, "session-a");
        changed.server_parameters = serde_json::json!({"cluster_id":"changed"});
        changed.transcript_digest = transcript_digest(
            &changed.server_parameters,
            &changed.management_client_csr_der_base64,
            &changed.management_client_identity_ref,
        )
        .unwrap();
        assert_eq!(
            state.claim(&changed, now).unwrap_err(),
            "claimed session presented a changed transcript"
        );
        let mut wrong = first;
        wrong.activation_code = "z".repeat(43);
        assert_eq!(
            state.claim(&wrong, now).unwrap_err(),
            "bootstrap authentication failed"
        );
    }

    #[test]
    fn explicit_preexisting_paths_are_rejected() {
        let directory = runtime_directory();
        let identity = BootstrapIdentity::generate().unwrap();
        let path = directory.path().join(ACTIVATION_FILE_NAME);
        fs::write(&path, b"sentinel").unwrap();
        let error = ActivationLease::create(
            directory.path(),
            "https://127.0.0.1:7443".into(),
            identity.endpoint_identity().into(),
            OffsetDateTime::UNIX_EPOCH,
            false,
        )
        .err()
        .unwrap();
        assert_eq!(error, "bootstrap activation path already exists");
        assert_eq!(fs::read(&path).unwrap(), b"sentinel");
        fs::remove_file(&path).unwrap();
        symlink("missing-target", &path).unwrap();
        let error = ActivationLease::create(
            directory.path(),
            "https://127.0.0.1:7443".into(),
            identity.endpoint_identity().into(),
            OffsetDateTime::UNIX_EPOCH,
            false,
        )
        .err()
        .unwrap();
        assert_eq!(error, "bootstrap activation path already exists");
        assert!(
            fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}
