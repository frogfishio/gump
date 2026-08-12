// SPDX-FileCopyrightText: 2026 Alexander R. Croft
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `gump` process entry — CLI + composed server roles (GUMP-N004 / C08).

use std::collections::BTreeSet;
use std::env;
use std::io::Read;
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine as _;
use gump_cli::{print_help, try_dispatch_cli};
use gump_connectors::{FakeObjectStore, RuntimeObjectStore, S3Config, S3ObjectStore};
use gump_crypto::{SignerEnrollment, SignerTrustPolicy, VerifyingKeyBytes};
use gump_memory::{ClusterJoinConfig, ClusterNetworkConfig};
use gump_server::accept::{AcceptStats, run_accept_loop};
use gump_server::bootstrap::{BootstrapEndpoint, empty_initialization_result};
use gump_server::compose::{InitOptions, ProductRuntime};
use gump_server::harden_daemon_startup;
use gump_server::management_bootstrap::{ManagementAuthority, ManagementBootstrapEndpoint};
use gump_server::roles::RoleSet;
use gump_transport::IdentityMaterial;
use gump_types::ClusterId;
use gump_types::{NodeId, Secret};
use serde::Deserialize;
use serde::Serialize;
use zeroize::Zeroize as _;

/// Process-wide cancel for SIGINT/SIGTERM (async-signal-safe store).
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("gump: {e}");
            ExitCode::from(2)
        }
    }
}

fn run(args: Vec<String>) -> Result<ExitCode, String> {
    if let Some(cli) = try_dispatch_cli(&args) {
        return cli;
    }
    if args.is_empty() {
        print_help();
        return Ok(ExitCode::SUCCESS);
    }
    match args[0].as_str() {
        "server" => run_server(&args[1..]),
        "cluster-material" => run_cluster_material(&args[1..]),
        other => Err(format!(
            "unknown command {other:?}; try gump run|test|server"
        )),
    }
}

#[derive(Serialize)]
struct MaterialDocument {
    schema: &'static str,
    cluster_id: String,
    nodes: Vec<MaterialNode>,
}

#[derive(Serialize)]
struct MaterialNode {
    ordinal: usize,
    node_id: String,
    certificate_der_hex: String,
    private_key_pkcs8_der_hex: String,
    ca_certificate_der_hex: String,
    join_token: Option<String>,
    allowed_join_tokens: Vec<MaterialJoinToken>,
}

#[derive(Serialize)]
struct MaterialJoinToken {
    node_id: String,
    token: String,
}

fn run_cluster_material(args: &[String]) -> Result<ExitCode, String> {
    let mut count = None;
    let mut cluster_id = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--nodes" => {
                i += 1;
                let raw = args.get(i).ok_or("--nodes needs an integer")?;
                count = Some(
                    raw.parse::<usize>()
                        .map_err(|_| "--nodes must be an integer")?,
                );
            }
            "--cluster-id" => {
                i += 1;
                let raw = args.get(i).ok_or("--cluster-id needs a UUID")?;
                cluster_id = Some(
                    raw.parse::<ClusterId>()
                        .map_err(|_| "--cluster-id must be a UUID")?,
                );
            }
            other => return Err(format!("unknown cluster-material arg: {other}")),
        }
        i += 1;
    }
    let count = count.ok_or("usage: gump cluster-material --nodes N [--cluster-id UUID]")?;
    if !(1..=7).contains(&count) {
        return Err("--nodes must be between 1 and 7".into());
    }
    let cluster_id = cluster_id.unwrap_or_else(ClusterId::new);
    let identities = (0..count)
        .map(|_| gump_transport::TransportIdentity {
            cluster_id,
            node_id: NodeId::new(),
            incarnation: gump_types::IncarnationId::new(),
            roles: vec![
                gump_transport::NodeRole::Memory,
                gump_transport::NodeRole::Agent,
                gump_transport::NodeRole::Controller,
                gump_transport::NodeRole::Ingress,
            ],
        })
        .collect::<Vec<_>>();
    let (materials, _) =
        gump_transport::mint_identity_set(identities).map_err(|e| e.to_string())?;
    let mut tokens = Vec::with_capacity(count.saturating_sub(1));
    for material in materials.iter().skip(1) {
        let mut token = [0u8; 32];
        getrandom::fill(&mut token).map_err(|e| format!("generate join token: {e}"))?;
        tokens.push((material.identity.node_id, hex_encode(&token)));
    }
    let mut nodes = Vec::with_capacity(count);
    for (index, material) in materials.into_iter().enumerate() {
        let join_token = index
            .checked_sub(1)
            .and_then(|i| tokens.get(i).map(|(_, token)| token.clone()));
        let allowed_join_tokens = if index == 0 {
            tokens
                .iter()
                .map(|(node_id, token)| MaterialJoinToken {
                    node_id: node_id.to_hyphenated(),
                    token: token.clone(),
                })
                .collect()
        } else {
            Vec::new()
        };
        nodes.push(MaterialNode {
            ordinal: index + 1,
            node_id: material.identity.node_id.to_hyphenated(),
            certificate_der_hex: hex_encode(material.certificate_der()),
            private_key_pkcs8_der_hex: hex_encode(material.private_key_der()),
            ca_certificate_der_hex: hex_encode(material.ca_certificate_der()),
            join_token,
            allowed_join_tokens,
        });
    }
    let output = serde_json::to_string(&MaterialDocument {
        schema: "gump.cluster-material/1",
        cluster_id: cluster_id.to_hyphenated(),
        nodes,
    })
    .map_err(|e| e.to_string())?;
    println!("{output}");
    Ok(ExitCode::SUCCESS)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn run_server(args: &[String]) -> Result<ExitCode, String> {
    let mut cfg = parse_server_args(args)?;

    // SECURITY §8 / STL-20: harden before any service work.
    let harden = harden_daemon_startup().map_err(|e| e.to_string())?;
    eprintln!("gump: process harden: {harden}");

    if cfg.mode == ServerMode::Bootstrap {
        return run_bootstrap_server(cfg);
    }

    let params = read_server_params(cfg.params_fd.take())?;
    run_configured_server(cfg, params, None)
}

#[derive(Debug)]
struct ConfiguredReady {
    cluster_id: String,
    node_id: String,
}

fn run_bootstrap_server(mut cfg: ServerConfig) -> Result<ExitCode, String> {
    let bind = cfg
        .bootstrap_bind
        .take()
        .ok_or("bootstrap mode requires --bootstrap-bind")?;
    let advertised = cfg
        .advertise_bootstrap
        .take()
        .ok_or("bootstrap mode requires --advertise-bootstrap")?;
    let management_bind = cfg
        .management_bind
        .take()
        .ok_or("bootstrap mode requires --management-bind")?;
    let management_endpoint = cfg
        .advertise_management
        .take()
        .ok_or("bootstrap mode requires --advertise-management")?;
    let endpoint = BootstrapEndpoint::bind(
        &bind,
        advertised,
        &cfg.runtime_directory,
        time::OffsetDateTime::now_utc(),
        cfg.require_tmpfs,
    )?;
    let activation_incarnation = endpoint.activation()?.incarnation.clone();
    eprintln!(
        "gump: bootstrap endpoint listening on {}",
        endpoint.local_address()?
    );
    let (claim_tx, claim_rx) = std::sync::mpsc::sync_channel(1);
    let result = empty_initialization_result();
    let endpoint_result = Arc::clone(&result);
    let endpoint_thread = std::thread::Builder::new()
        .name("gump-bootstrap".into())
        .spawn(move || endpoint.serve(claim_tx, endpoint_result))
        .map_err(|e| format!("start bootstrap endpoint: {e}"))?;
    let mut request = claim_rx
        .recv()
        .map_err(|_| "bootstrap endpoint stopped before initialization")?;
    let node_id = match request
        .server_parameters
        .get("cluster_transport")
        .and_then(|transport| transport.get("certificate_der_hex"))
        .and_then(serde_json::Value::as_str)
        .and_then(node_id_from_certificate_hex)
    {
        Some(node_id) => node_id,
        None => {
            return reject_bootstrap_claim(
                &result,
                endpoint_thread,
                "bootstrap initialization requires valid cluster_transport identity material"
                    .into(),
            );
        }
    };
    let params: ServerParams =
        match serde_json::from_value(std::mem::take(&mut request.server_parameters)) {
            Ok(parameters) => parameters,
            Err(error) => {
                return reject_bootstrap_claim(
                    &result,
                    endpoint_thread,
                    format!("invalid bootstrap server parameters: {error}"),
                );
            }
        };
    cfg.mode = ServerMode::Init;
    cfg.params_fd = None;
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    let runtime_thread = std::thread::Builder::new()
        .name("gump-runtime".into())
        .spawn(move || run_configured_server(cfg, Some(params), Some((ready_tx, node_id))))
        .map_err(|e| format!("start initialized runtime: {e}"))?;
    let ready = match ready_rx.recv() {
        Ok(ready) => ready,
        Err(_) => {
            let failure = runtime_thread
                .join()
                .map_err(|_| "initialized runtime panicked")?
                .err()
                .unwrap_or_else(|| "initialized runtime exited before readiness".into());
            set_initialization_result(&result, Err(failure))?;
            endpoint_thread
                .join()
                .map_err(|_| "bootstrap endpoint panicked")??;
            return Ok(ExitCode::from(1));
        }
    };
    let management_authority = ManagementAuthority::generate()?;
    let management_client_certificate =
        management_authority.issue_client(&request.management_client_csr_der_base64)?;
    request.activation_code.zeroize();
    let management_server = ManagementBootstrapEndpoint::bind(
        &management_bind,
        &management_authority,
        request.session_id.clone(),
        Arc::clone(&result),
        ready.cluster_id.clone(),
        ready.node_id.clone(),
    )?;
    std::thread::Builder::new()
        .name("gump-management".into())
        .spawn(move || {
            if let Err(error) = management_server.serve() {
                eprintln!("gump: management endpoint stopped: {error}");
            }
        })
        .map_err(|e| format!("start management endpoint: {e}"))?;
    let committed = gump_protocol::bootstrap::BootstrapResult {
        schema: gump_protocol::bootstrap::RESULT_SCHEMA.into(),
        status: "committed".into(),
        cluster_identity: ready.cluster_id,
        node_identity: ready.node_id,
        session_id: request.session_id.clone(),
        committed_incarnation: activation_incarnation,
        management_endpoint,
        management_client_identity_ref: request.management_client_identity_ref.clone(),
        management_ca_certificate_der_base64: base64::engine::general_purpose::STANDARD
            .encode(management_authority.ca_der()),
        management_client_certificate_der_base64: base64::engine::general_purpose::STANDARD
            .encode(management_client_certificate),
        management_mtls_verified: false,
        node_admitted: true,
        activation_consumed: false,
        bootstrap_closed: false,
    };
    set_initialization_result(&result, Ok(committed))?;
    endpoint_thread
        .join()
        .map_err(|_| "bootstrap endpoint panicked")??;
    runtime_thread
        .join()
        .map_err(|_| "initialized runtime panicked")?
}

fn set_initialization_result(
    shared: &gump_server::bootstrap::InitializationResult,
    value: Result<gump_protocol::bootstrap::BootstrapResult, String>,
) -> Result<(), String> {
    let (lock, ready) = &**shared;
    let mut slot = lock.lock().map_err(|_| "bootstrap result poisoned")?;
    *slot = Some(value);
    ready.notify_all();
    Ok(())
}

fn reject_bootstrap_claim(
    result: &gump_server::bootstrap::InitializationResult,
    endpoint: std::thread::JoinHandle<Result<(), String>>,
    message: String,
) -> Result<ExitCode, String> {
    set_initialization_result(result, Err(message.clone()))?;
    endpoint
        .join()
        .map_err(|_| "bootstrap endpoint panicked")??;
    Err(message)
}

fn node_id_from_certificate_hex(hex: &str) -> Option<String> {
    use x509_parser::extensions::{GeneralName, ParsedExtension};
    use x509_parser::prelude::FromDer as _;

    let bytes = parse_hex_vec(hex, "cluster certificate", 1024 * 1024).ok()?;
    let (_, certificate) = x509_parser::certificate::X509Certificate::from_der(&bytes).ok()?;
    let sans = certificate.extensions().iter().find_map(|extension| {
        match extension.parsed_extension() {
            ParsedExtension::SubjectAlternativeName(names) => Some(names),
            _ => None,
        }
    })?;
    sans.general_names.iter().find_map(|name| match name {
        GeneralName::DNSName(value) => value
            .strip_prefix("gump.node.")
            .and_then(|value| value.parse::<NodeId>().ok())
            .map(|node| node.to_hyphenated()),
        _ => None,
    })
}

fn run_configured_server(
    cfg: ServerConfig,
    mut params: Option<ServerParams>,
    ready: Option<(std::sync::mpsc::SyncSender<ConfiguredReady>, String)>,
) -> Result<ExitCode, String> {
    let uid = unsafe { libc::geteuid() } as u32;
    let bootstrap_initialization = ready.is_some();
    let recovery = params.as_mut().and_then(|parameters| {
        parameters.recovery_secret_hex.take().map(|secret| {
            (
                secret,
                parameters
                    .recovery_key_id
                    .take()
                    .unwrap_or_else(|| "cluster-software-1".into()),
            )
        })
    });
    let configured_cluster_id = params
        .as_ref()
        .and_then(|p| p.cluster_id.as_deref())
        .map(str::parse::<ClusterId>)
        .transpose()
        .map_err(|_| "params.cluster_id must be a UUIDv7".to_string())?;
    let (cluster_network, transport_cluster_id, controller_holder) =
        build_cluster_network(&cfg, params.as_mut())?;
    let cluster_id = match (configured_cluster_id, transport_cluster_id) {
        (Some(a), Some(b)) if a != b => {
            return Err("params.cluster_id differs from mTLS certificate cluster identity".into());
        }
        (Some(id), _) | (_, Some(id)) => Some(id),
        (None, None) => None,
    };
    let signer_trust = build_signer_trust(params.as_ref())?;
    let object_store = build_object_store(&cfg, params.as_mut())?;
    let runtime = ProductRuntime::init_with_runtime(
        InitOptions {
            cluster_id,
            roles: cfg.roles,
            peer_uid: uid,
            controller_holder,
            object_store,
            signer_trust,
        },
        cfg.state_root.clone(),
        cluster_network,
    )?;
    if let Some((mut recovery_secret_hex, key_id)) = recovery {
        let parsed = parse_hex_vec(&recovery_secret_hex, "recovery secret", 32);
        recovery_secret_hex.zeroize();
        let mut bytes = parsed?;
        let mut secret_bytes: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| "recovery secret must be exactly 32 bytes")?;
        bytes.zeroize();
        let recovery_secret = gump_crypto::RecoverySecret::from_bytes(secret_bytes);
        secret_bytes.zeroize();
        let custody = runtime
            .local_api
            .custody
            .as_ref()
            .ok_or("recovery secret supplied but custody role is disabled")?;
        custody
            .lock()
            .map_err(|_| "custody lock poisoned")?
            .activate_software_1of1(&recovery_secret, key_id)
            .map_err(|e| format!("activate bootstrap custody: {e}"))?;
    } else if bootstrap_initialization && runtime.local_api.custody.is_some() {
        return Err("bootstrap initialization requires recovery_secret_hex".into());
    }
    eprintln!("gump: {}", runtime.status_line());

    if let Some(parent) = cfg.socket.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_file(&cfg.socket);

    let listener = UnixListener::bind(&cfg.socket).map_err(|e| e.to_string())?;
    eprintln!("gump: listening on {}", cfg.socket.display());

    if let Some((sender, node_id)) = ready {
        sender
            .send(ConfiguredReady {
                cluster_id: runtime.cluster_id.to_hyphenated(),
                node_id,
            })
            .map_err(|_| "bootstrap initializer stopped before runtime readiness")?;
    }

    install_signal_handlers();

    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_watch = Arc::clone(&cancel);
    std::thread::spawn(move || {
        while !SHUTDOWN.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        cancel_watch.store(true, Ordering::SeqCst);
    });

    let daemon = Arc::new(runtime.local_api);
    if let (Some(execution), Some(cluster), Some(store)) = (
        runtime.execution,
        daemon.memory_cluster.clone(),
        daemon.object_store.clone(),
    ) {
        let reconcile_cancel = Arc::clone(&cancel);
        std::thread::Builder::new()
            .name("gump-reconcile".into())
            .spawn(move || {
                while !reconcile_cancel.load(Ordering::SeqCst) {
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    if let Ok(mut loop_) = execution.lock() {
                        if let Err(e) = loop_.reconcile(&cluster, &store, now_ms) {
                            loop_.note_error(e);
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            })
            .map_err(|e| format!("start reconcile loop: {e}"))?;
    }
    let stats = AcceptStats::new();
    run_accept_loop(Arc::clone(&daemon), listener, cancel, stats).map_err(|e| e.to_string())?;
    if let Some(cluster) = &daemon.memory_cluster {
        let _ = cluster.shutdown();
    }
    eprintln!("gump: shutdown complete");
    Ok(ExitCode::SUCCESS)
}

struct ServerConfig {
    mode: ServerMode,
    socket: PathBuf,
    roles: RoleSet,
    memory_object_store: bool,
    params_fd: Option<i32>,
    state_root: PathBuf,
    join_seed: Option<std::net::SocketAddr>,
    bootstrap_bind: Option<String>,
    advertise_bootstrap: Option<String>,
    runtime_directory: PathBuf,
    require_tmpfs: bool,
    management_bind: Option<String>,
    advertise_management: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ServerMode {
    Init,
    Join,
    Bootstrap,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerParams {
    cluster_id: Option<String>,
    recovery_secret_hex: Option<String>,
    recovery_key_id: Option<String>,
    s3: Option<S3Params>,
    #[serde(default)]
    release_signers: Vec<ReleaseSignerParams>,
    cluster_transport: Option<ClusterTransportParams>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClusterTransportParams {
    bind: String,
    advertise: String,
    certificate_der_hex: String,
    private_key_pkcs8_der_hex: String,
    ca_certificate_der_hex: String,
    join_token: Option<String>,
    #[serde(default)]
    allowed_join_tokens: Vec<AllowedJoinTokenParams>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AllowedJoinTokenParams {
    node_id: String,
    token: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseSignerParams {
    public_key_hex: String,
    namespaces: Vec<String>,
    expires_at_ms: Option<u64>,
    #[serde(default)]
    capabilities: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct S3Params {
    endpoint: String,
    region: String,
    bucket: String,
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
    #[serde(default = "yes")]
    force_path_style: bool,
}

const fn yes() -> bool {
    true
}

fn parse_server_args(args: &[String]) -> Result<ServerConfig, String> {
    let mut init = false;
    let mut bootstrap = false;
    let mut socket = PathBuf::from("/tmp/gump.sock");
    let mut roles = RoleSet::default_init();
    let mut memory_object_store = false;
    let mut params_fd = None;
    let mut state_root = PathBuf::from("/var/lib/gump");
    let mut join_seed = None;
    let mut bootstrap_bind = None;
    let mut advertise_bootstrap = None;
    let mut runtime_directory = PathBuf::from("/run/gump");
    let mut require_tmpfs = true;
    let mut management_bind = None;
    let mut advertise_management = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--init" => init = true,
            "--bootstrap" => bootstrap = true,
            "--join" => {
                i += 1;
                let seed = args.get(i).ok_or("--join needs seed host:port")?;
                join_seed = Some(seed.parse().map_err(|_| "--join must be an IP:port")?);
            }
            "--memory-object-store" => memory_object_store = true,
            "--bootstrap-bind" => {
                i += 1;
                bootstrap_bind = Some(args.get(i).ok_or("--bootstrap-bind needs IP:port")?.clone());
            }
            "--advertise-bootstrap" => {
                i += 1;
                advertise_bootstrap = Some(
                    args.get(i)
                        .ok_or("--advertise-bootstrap needs an https origin")?
                        .clone(),
                );
            }
            "--management-bind" => {
                i += 1;
                management_bind = Some(
                    args.get(i)
                        .ok_or("--management-bind needs IP:port")?
                        .clone(),
                );
            }
            "--advertise-management" => {
                i += 1;
                advertise_management = Some(
                    args.get(i)
                        .ok_or("--advertise-management needs an https origin")?
                        .clone(),
                );
            }
            "--runtime-directory" => {
                i += 1;
                runtime_directory =
                    PathBuf::from(args.get(i).ok_or("--runtime-directory needs a path")?);
            }
            "--allow-non-tmpfs-for-test" => require_tmpfs = false,
            "--params-fd" => {
                i += 1;
                let raw = args.get(i).ok_or("--params-fd needs a descriptor")?;
                let fd: i32 = raw.parse().map_err(|_| "--params-fd must be an integer")?;
                if fd < 3 {
                    return Err("--params-fd must be an inherited descriptor >= 3".into());
                }
                params_fd = Some(fd);
            }
            "--socket" => {
                i += 1;
                socket = PathBuf::from(args.get(i).ok_or("--socket needs a path")?);
            }
            "--state-root" => {
                i += 1;
                state_root = PathBuf::from(args.get(i).ok_or("--state-root needs a path")?);
            }
            "--role" | "--roles" => {
                i += 1;
                let spec = args.get(i).ok_or("--role needs a CSV list")?;
                roles = RoleSet::from_csv(spec)?;
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unknown server arg: {other}")),
        }
        i += 1;
    }
    let selected = usize::from(init) + usize::from(join_seed.is_some()) + usize::from(bootstrap);
    if selected != 1 {
        return Err(
            "gump server requires exactly one of --init, --join <seed>, or --bootstrap".into(),
        );
    }
    let mode = if bootstrap {
        ServerMode::Bootstrap
    } else if init {
        ServerMode::Init
    } else {
        ServerMode::Join
    };
    if mode == ServerMode::Bootstrap && params_fd.is_some() {
        return Err(
            "bootstrap mode receives initialization only through its pinned endpoint".into(),
        );
    }
    if memory_object_store && params_fd.is_some() {
        return Err("--memory-object-store and --params-fd are mutually exclusive".into());
    }
    Ok(ServerConfig {
        mode,
        socket,
        roles,
        memory_object_store,
        params_fd,
        state_root,
        join_seed,
        bootstrap_bind,
        advertise_bootstrap,
        runtime_directory,
        require_tmpfs,
        management_bind,
        advertise_management,
    })
}

fn read_server_params(fd: Option<i32>) -> Result<Option<ServerParams>, String> {
    let Some(fd) = fd else { return Ok(None) };
    // SAFETY: parse_server_args requires an inherited, non-stdio descriptor.
    // Ownership is intentionally consumed exactly once by server startup.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut body = Vec::new();
    file.by_ref()
        .take(64 * 1024 + 1)
        .read_to_end(&mut body)
        .map_err(|e| format!("read --params-fd: {e}"))?;
    if body.len() > 64 * 1024 {
        return Err("server params exceed 64 KiB".into());
    }
    let parsed = serde_json::from_slice(&body);
    body.fill(0);
    Ok(Some(
        parsed.map_err(|e| format!("invalid server params: {e}"))?,
    ))
}

fn build_object_store(
    cfg: &ServerConfig,
    params: Option<&mut ServerParams>,
) -> Result<Option<RuntimeObjectStore>, String> {
    if cfg.memory_object_store {
        return Ok(Some(RuntimeObjectStore::Memory(FakeObjectStore::new())));
    }
    let Some(s3) = params.and_then(|p| p.s3.as_mut()) else {
        return Ok(None);
    };
    let store = S3ObjectStore::new(S3Config {
        endpoint: s3.endpoint.clone(),
        region: s3.region.clone(),
        bucket: s3.bucket.clone(),
        access_key_id: Some(s3.access_key_id.clone()),
        secret_access_key: Some(gump_types::Secret::new(std::mem::take(
            &mut s3.secret_access_key,
        ))),
        session_token: s3.session_token.take().map(gump_types::Secret::new),
        force_path_style: s3.force_path_style,
        require_conditional_copy: true,
    })
    .map_err(|e| format!("initialize S3 Capsule store: {e}"))?;
    Ok(Some(RuntimeObjectStore::S3(store)))
}

fn build_cluster_network(
    cfg: &ServerConfig,
    params: Option<&mut ServerParams>,
) -> Result<(Option<ClusterNetworkConfig>, Option<ClusterId>, u64), String> {
    let Some(transport) = params.and_then(|p| p.cluster_transport.as_mut()) else {
        if cfg.join_seed.is_some() {
            return Err("--join requires params.cluster_transport mTLS material".into());
        }
        return Ok((None, None, 1));
    };
    let cert = parse_hex_vec(
        &std::mem::take(&mut transport.certificate_der_hex),
        "cluster certificate",
        1024 * 1024,
    )?;
    let key = parse_hex_vec(
        &std::mem::take(&mut transport.private_key_pkcs8_der_hex),
        "cluster private key",
        1024 * 1024,
    )?;
    let ca = parse_hex_vec(
        &std::mem::take(&mut transport.ca_certificate_der_hex),
        "cluster CA certificate",
        1024 * 1024,
    )?;
    let (material, trust) =
        IdentityMaterial::from_der(cert, key, ca).map_err(|e| format!("cluster mTLS: {e}"))?;
    let cluster_id = material.identity.cluster_id;
    if !material
        .identity
        .roles
        .contains(&gump_transport::NodeRole::Memory)
    {
        return Err("cluster certificate lacks memory role".into());
    }
    let holder = memory_node_id(material.identity.node_id);
    let bind = transport
        .bind
        .parse()
        .map_err(|_| "cluster_transport.bind must be IP:port")?;
    let advertise = transport
        .advertise
        .parse()
        .map_err(|_| "cluster_transport.advertise must be IP:port")?;
    let mut allowed = std::collections::BTreeMap::new();
    for enrollment in &mut transport.allowed_join_tokens {
        let node: NodeId = enrollment
            .node_id
            .parse()
            .map_err(|_| "allowed join node_id must be UUIDv7")?;
        if allowed
            .insert(
                memory_node_id(node),
                Secret::new(std::mem::take(&mut enrollment.token)),
            )
            .is_some()
        {
            return Err("duplicate allowed join node_id".into());
        }
    }
    let join = match cfg.join_seed {
        Some(seed) => Some(ClusterJoinConfig {
            seed,
            token: Secret::new(
                transport
                    .join_token
                    .take()
                    .ok_or("joining node requires cluster_transport.join_token")?,
            ),
        }),
        None => {
            if transport.join_token.is_some() {
                return Err("seed must not specify cluster_transport.join_token".into());
            }
            None
        }
    };
    Ok((
        Some(ClusterNetworkConfig {
            bind,
            advertise,
            material,
            trust,
            join_tokens: allowed,
            join,
        }),
        Some(cluster_id),
        holder,
    ))
}

fn memory_node_id(node_id: NodeId) -> u64 {
    u64::from_be_bytes(
        node_id.as_bytes()[8..16]
            .try_into()
            .expect("NodeId suffix is 8 bytes"),
    )
}

fn parse_hex_vec(hex: &str, label: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    if hex.is_empty() || hex.len() % 2 != 0 || hex.len() / 2 > max_bytes {
        return Err(format!(
            "{label} hex is empty, odd, or exceeds {max_bytes} bytes"
        ));
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    for chunk in hex.as_bytes().chunks_exact(2) {
        let pair = std::str::from_utf8(chunk).map_err(|_| format!("{label} is not UTF-8 hex"))?;
        out.push(u8::from_str_radix(pair, 16).map_err(|_| format!("{label} contains bad hex"))?);
    }
    Ok(out)
}

fn build_signer_trust(params: Option<&ServerParams>) -> Result<SignerTrustPolicy, String> {
    let mut trust = SignerTrustPolicy::new();
    for signer in params.into_iter().flat_map(|p| &p.release_signers) {
        if signer.namespaces.is_empty() {
            return Err("release signer must authorize at least one namespace".into());
        }
        let public_key = parse_hex32(&signer.public_key_hex, "release signer public key")?;
        trust
            .enroll(SignerEnrollment {
                public_key: VerifyingKeyBytes(public_key),
                namespaces: signer.namespaces.iter().cloned().collect::<BTreeSet<_>>(),
                expires_at_ms: signer.expires_at_ms,
                capabilities: signer.capabilities.iter().cloned().collect::<BTreeSet<_>>(),
            })
            .map_err(|e| format!("enroll release signer: {e}"))?;
    }
    Ok(trust)
}

fn parse_hex32(value: &str, label: &str) -> Result<[u8; 32], String> {
    if value.len() != 64 {
        return Err(format!("{label} must contain 64 lowercase hex characters"));
    }
    let mut out = [0u8; 32];
    for (i, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let s = std::str::from_utf8(pair).map_err(|_| format!("invalid {label}"))?;
        out[i] = u8::from_str_radix(s, 16).map_err(|_| format!("invalid {label}"))?;
    }
    Ok(out)
}

fn install_signal_handlers() {
    unsafe {
        libc::signal(libc::SIGINT, handle_signal as usize);
        libc::signal(libc::SIGTERM, handle_signal as usize);
    }
}

extern "C" fn handle_signal(_: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}
