//! `gump` process entry — CLI + composed server roles (GUMP-N004 / C08).

use std::env;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gump_cli::{print_help, try_dispatch_cli};
use gump_server::accept::{AcceptStats, run_accept_loop};
use gump_server::compose::{InitOptions, ProductRuntime};
use gump_server::harden_daemon_startup;
use gump_server::roles::RoleSet;

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
        other => Err(format!(
            "unknown command {other:?}; try gump run|test|server"
        )),
    }
}

fn run_server(args: &[String]) -> Result<ExitCode, String> {
    let cfg = parse_server_args(args)?;

    // SECURITY §8 / STL-20: harden before any service work.
    let harden = harden_daemon_startup().map_err(|e| e.to_string())?;
    eprintln!("gump: process harden: {harden}");

    let uid = unsafe { libc::geteuid() } as u32;
    let runtime = ProductRuntime::init(InitOptions {
        roles: cfg.roles,
        peer_uid: uid,
        controller_holder: 1,
    })?;
    eprintln!("gump: {}", runtime.status_line());

    if let Some(parent) = cfg.socket.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_file(&cfg.socket);

    let listener = UnixListener::bind(&cfg.socket).map_err(|e| e.to_string())?;
    eprintln!("gump: listening on {}", cfg.socket.display());

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
    let stats = AcceptStats::new();
    run_accept_loop(daemon, listener, cancel, stats).map_err(|e| e.to_string())?;
    eprintln!("gump: shutdown complete");
    Ok(ExitCode::SUCCESS)
}

struct ServerConfig {
    socket: PathBuf,
    roles: RoleSet,
}

fn parse_server_args(args: &[String]) -> Result<ServerConfig, String> {
    let mut init = false;
    let mut socket = PathBuf::from("/tmp/gump.sock");
    let mut roles = RoleSet::default_init();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--init" => init = true,
            "--socket" => {
                i += 1;
                socket = PathBuf::from(args.get(i).ok_or("--socket needs a path")?);
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
    if !init {
        return Err("gump server requires --init in v1 (GUMP-N004)".into());
    }
    Ok(ServerConfig { socket, roles })
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
