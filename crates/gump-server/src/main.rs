//! `gump` server binary — local Unix API listener scaffold (C08).

use std::env;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::ExitCode;

use gump_server::peer::{peer_cred_of, PeerAllowlist};
use gump_server::serve::{bootstrap_controller, serve_connection, LocalDaemon};
use gump_server::harden_daemon_startup;

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("gump-server: {e}");
            ExitCode::from(2)
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    // SECURITY §8 / STL-20: harden before any service work (fail closed when policy requires).
    let harden = harden_daemon_startup().map_err(|e| e.to_string())?;
    eprintln!("gump-server: process harden: {harden}");

    let socket = parse_socket(&args)?;
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_file(&socket);

    let uid = unsafe { libc::geteuid() } as u32;
    let mut daemon = LocalDaemon::new(PeerAllowlist::same_uid(uid));
    bootstrap_controller(&mut daemon, 1, 0);

    let listener = UnixListener::bind(&socket).map_err(|e| e.to_string())?;
    eprintln!("gump-server: listening on {}", socket.display());

    // Single-accept demo loop for local API (production accept loop lands with C08 ops expansion).
    let (mut stream, _) = listener.accept().map_err(|e| e.to_string())?;
    let peer = peer_cred_of(&stream).map_err(|e| e.to_string())?;
    let _ = serve_connection(&daemon, peer, &mut stream).map_err(|e| e.to_string())?;
    Ok(())
}

fn parse_socket(args: &[String]) -> Result<PathBuf, String> {
    let mut socket = PathBuf::from("/tmp/gump.sock");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--socket" => {
                i += 1;
                socket = PathBuf::from(args.get(i).ok_or("--socket needs a path")?);
            }
            "-h" | "--help" => {
                eprintln!("usage: server [--socket PATH]");
                std::process::exit(0);
            }
            other => return Err(format!("unknown arg: {other}")),
        }
        i += 1;
    }
    Ok(socket)
}
