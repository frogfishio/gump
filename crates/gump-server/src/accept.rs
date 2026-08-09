//! Cancellable concurrent accept loop for the local Unix API (GUMP-N004 / C08).

use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::peer::peer_cred_of;
use crate::serve::{LocalDaemon, ServeError, serve_connection};

/// The local control socket is deliberately small and bounded. A local peer
/// must not be able to create an unbounded number of daemon threads.
pub const MAX_ACTIVE_CONNECTIONS: usize = 64;
pub const CONNECTION_IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Shared cancel flag for graceful shutdown of the accept loop.
pub type CancelFlag = Arc<AtomicBool>;

pub fn new_cancel_flag() -> CancelFlag {
    Arc::new(AtomicBool::new(false))
}

#[derive(Clone, Debug)]
pub struct AcceptStats {
    pub accepted: Arc<AtomicUsize>,
    pub active: Arc<AtomicUsize>,
    pub errors: Arc<AtomicUsize>,
}

impl AcceptStats {
    pub fn new() -> Self {
        Self {
            accepted: Arc::new(AtomicUsize::new(0)),
            active: Arc::new(AtomicUsize::new(0)),
            errors: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Default for AcceptStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Run accept until `cancel` is set, serving each connection on a worker thread.
///
/// Returns after the listener stops accepting and in-flight workers join.
pub fn run_accept_loop(
    daemon: Arc<LocalDaemon>,
    listener: UnixListener,
    cancel: CancelFlag,
    stats: AcceptStats,
) -> Result<(), io::Error> {
    listener.set_nonblocking(true)?;
    let mut workers: Vec<JoinHandle<()>> = Vec::new();

    while !cancel.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                stats.accepted.fetch_add(1, Ordering::SeqCst);
                if stats.active.load(Ordering::SeqCst) >= MAX_ACTIVE_CONNECTIONS {
                    stats.errors.fetch_add(1, Ordering::SeqCst);
                    drop(stream);
                    continue;
                }
                stats.active.fetch_add(1, Ordering::SeqCst);
                let daemon = Arc::clone(&daemon);
                let active = Arc::clone(&stats.active);
                let errors = Arc::clone(&stats.errors);
                workers.push(thread::spawn(move || {
                    if let Err(e) = serve_unix_stream(&daemon, stream) {
                        let _ = e;
                        errors.fetch_add(1, Ordering::SeqCst);
                    }
                    active.fetch_sub(1, Ordering::SeqCst);
                }));
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(15));
            }
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                return Err(e);
            }
        }
        workers.retain(|h| !h.is_finished());
    }

    for h in workers {
        let _ = h.join();
    }
    Ok(())
}

fn serve_unix_stream(daemon: &LocalDaemon, mut stream: UnixStream) -> Result<(), ServeError> {
    stream
        .set_nonblocking(false)
        .map_err(|e| ServeError::Frame(crate::framing::FrameError::Io(e.to_string())))?;
    stream
        .set_read_timeout(Some(CONNECTION_IO_TIMEOUT))
        .map_err(|e| ServeError::Frame(crate::framing::FrameError::Io(e.to_string())))?;
    stream
        .set_write_timeout(Some(CONNECTION_IO_TIMEOUT))
        .map_err(|e| ServeError::Frame(crate::framing::FrameError::Io(e.to_string())))?;
    let peer = peer_cred_of(&stream).map_err(ServeError::from)?;
    serve_connection(daemon, peer, &mut stream)?;
    Ok(())
}
