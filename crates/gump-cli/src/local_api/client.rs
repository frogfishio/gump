//! Unix-domain client for the local daemon API (GUMP-N006).
//!
//! CLI verbs must call this client — they must not reimplement server semantics.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::framing::{FrameError, read_frame, write_frame};
use super::machine::{LocalCall, LocalRequest, LocalResponse, MachineOutputV1};

#[derive(Clone, Debug)]
pub struct LocalClient {
    socket: PathBuf,
    /// Soft reconnect attempts after connection refused / broken pipe.
    max_reconnects: u32,
}

#[derive(Clone, Debug)]
pub enum LocalClientError {
    Io(String),
    Frame(FrameError),
    Json(String),
    Protocol(LocalResponse),
    Interrupted,
}

impl std::fmt::Display for LocalClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Frame(e) => write!(f, "{e}"),
            Self::Json(e) => write!(f, "json: {e}"),
            Self::Protocol(LocalResponse::Error(e)) => {
                write!(f, "{} ({})", e.code, e.reason)
            }
            Self::Protocol(other) => write!(f, "unexpected protocol body {other:?}"),
            Self::Interrupted => write!(f, "interrupted"),
        }
    }
}

impl std::error::Error for LocalClientError {}

impl LocalClient {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
            max_reconnects: 3,
        }
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// One request/response exchange with optional deadline and reconnect.
    pub fn call(
        &self,
        request: LocalRequest,
        deadline: Option<Duration>,
    ) -> Result<LocalResponse, LocalClientError> {
        let mut call = LocalCall::new(request);
        if let Some(d) = deadline {
            call.deadline_ms = Some(d.as_millis() as u64);
        }
        self.call_raw(call)
    }

    pub fn call_raw(&self, call: LocalCall) -> Result<LocalResponse, LocalClientError> {
        let mut last_err = LocalClientError::Io("no attempts".into());
        for attempt in 0..=self.max_reconnects {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(25 * u64::from(attempt)));
            }
            match self.exchange_once(&call) {
                Ok(body) => return Ok(body),
                Err(LocalClientError::Interrupted) => return Err(LocalClientError::Interrupted),
                Err(LocalClientError::Protocol(e)) => return Err(LocalClientError::Protocol(e)),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    /// Signal cancellation to the daemon (lifecycle contract).
    pub fn cancel(&self, subject: impl Into<String>) -> Result<LocalResponse, LocalClientError> {
        let mut call = LocalCall::new(LocalRequest::Lifecycle {
            action: "cancel".into(),
            subject: subject.into(),
        });
        call.cancelled = true;
        self.call_raw(call)
    }

    fn exchange_once(&self, call: &LocalCall) -> Result<LocalResponse, LocalClientError> {
        let mut stream = UnixStream::connect(&self.socket)
            .map_err(|e| LocalClientError::Io(format!("connect {}: {e}", self.socket.display())))?;
        if let Some(ms) = call.deadline_ms {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(ms.max(1))));
            let _ = stream.set_write_timeout(Some(Duration::from_millis(ms.max(1))));
        }
        let bytes = serde_json::to_vec(call).map_err(|e| LocalClientError::Json(e.to_string()))?;
        write_frame(&mut stream, &bytes).map_err(LocalClientError::Frame)?;
        let frame = read_frame(&mut stream).map_err(|e| match e {
            FrameError::Io(msg) if msg.contains("Interrupted") => LocalClientError::Interrupted,
            other => LocalClientError::Frame(other),
        })?;
        let out: MachineOutputV1 =
            serde_json::from_slice(&frame).map_err(|e| LocalClientError::Json(e.to_string()))?;
        if let LocalResponse::Error(ref err) = out.body {
            if err.code == "PROTOCOL_MISMATCH"
                || err.code == "UNAUTHORIZED"
                || err.code == "DEADLINE_EXCEEDED"
                || err.code == "CANCELLED"
            {
                return Err(LocalClientError::Protocol(out.body));
            }
        }
        Ok(out.body)
    }
}
