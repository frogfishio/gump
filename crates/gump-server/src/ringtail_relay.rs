//! Bounded node-local relay for the `gump-ringtail/1` integration profile.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gump_types::Secret;
use serde_json::json;

const QUEUE_RECORDS: usize = 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(250);
const IO_TIMEOUT: Duration = Duration::from_millis(500);
const MAX_RESPONSE_BYTES: usize = 8 * 1024;

pub(crate) struct RelayTarget {
    pub address: SocketAddr,
    pub path: String,
    pub token: Secret<String>,
}

#[derive(Clone)]
pub(crate) struct RingtailRelay {
    tx: SyncSender<RelayEvent>,
    target: Arc<Mutex<Option<RelayTarget>>>,
    counters: Arc<RelayCounters>,
}

#[derive(Default)]
struct RelayCounters {
    accepted: AtomicU64,
    failed: AtomicU64,
    dropped: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RelayStats {
    pub active: bool,
    pub accepted: u64,
    pub failed: u64,
    pub dropped: u64,
}

struct RelayEvent {
    topic: &'static str,
    sequence: u64,
    bytes: Vec<u8>,
    cluster: String,
    node: String,
    attempt: String,
}

impl RingtailRelay {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::sync_channel::<RelayEvent>(QUEUE_RECORDS);
        let target = Arc::new(Mutex::new(None));
        let worker_target = Arc::clone(&target);
        let counters = Arc::new(RelayCounters::default());
        let worker_counters = Arc::clone(&counters);
        let _ = std::thread::Builder::new()
            .name("gump-ringtail-relay".into())
            .spawn(move || {
                while let Ok(event) = rx.recv() {
                    let guard = match worker_target.lock() {
                        Ok(guard) => guard,
                        Err(_) => continue,
                    };
                    let Some(target) = guard.as_ref() else {
                        continue;
                    };
                    match deliver(target, &event) {
                        Ok(()) => {
                            worker_counters.accepted.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(_) => {
                            worker_counters.failed.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            });
        Self {
            tx,
            target,
            counters,
        }
    }

    pub fn set_target(&self, target: Option<RelayTarget>) {
        if let Ok(mut slot) = self.target.lock() {
            *slot = target;
        }
    }

    pub fn try_emit(
        &self,
        topic: &'static str,
        sequence: u64,
        bytes: &[u8],
        cluster: String,
        node: String,
        attempt: String,
    ) {
        let event = RelayEvent {
            topic,
            sequence,
            bytes: bytes.to_vec(),
            cluster,
            node,
            attempt,
        };
        match self.tx.try_send(event) {
            Ok(()) => {}
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.counters.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn stats(&self) -> RelayStats {
        RelayStats {
            active: self
                .target
                .lock()
                .map(|target| target.is_some())
                .unwrap_or(false),
            accepted: self.counters.accepted.load(Ordering::Relaxed),
            failed: self.counters.failed.load(Ordering::Relaxed),
            dropped: self.counters.dropped.load(Ordering::Relaxed),
        }
    }
}

fn deliver(target: &RelayTarget, event: &RelayEvent) -> Result<(), String> {
    let message = String::from_utf8_lossy(&event.bytes);
    let envelope = json!({
        "profile": "gump.ratatouille/1",
        "topic": event.topic,
        "gump": {
            "cluster": event.cluster,
            "node": event.node,
            "attempt": event.attempt,
        },
        "record": {
            "seq": event.sequence,
            "topic": event.topic,
            "src": {},
            "args": [message],
        }
    });
    let mut body = serde_json::to_vec(&envelope).map_err(|e| e.to_string())?;
    body.push(b'\n');
    let mut stream =
        TcpStream::connect_timeout(&target.address, CONNECT_TIMEOUT).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|e| e.to_string())?;
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        target.path,
        target.address,
        target.token.expose(),
        body.len(),
    );
    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.write_all(&body))
        .map_err(|e| e.to_string())?;
    let mut response = Vec::new();
    stream
        .take(MAX_RESPONSE_BYTES as u64)
        .read_to_end(&mut response)
        .map_err(|e| e.to_string())?;
    let status = std::str::from_utf8(&response)
        .ok()
        .and_then(|text| text.lines().next())
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|raw| raw.parse::<u16>().ok())
        .unwrap_or(0);
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(format!("Ringtail sink returned HTTP {status}"))
    }
}
