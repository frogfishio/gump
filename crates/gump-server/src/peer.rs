//! Peer credentials for Unix-domain connections (DECISIONS D007).

use std::io;

/// Credentials of the peer process on a Unix-domain socket.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct PeerCred {
    pub uid: u32,
    pub gid: u32,
    pub pid: Option<u32>,
}

impl PeerCred {
    pub fn new(uid: u32, gid: u32, pid: Option<u32>) -> Self {
        Self { uid, gid, pid }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeerAuthError {
    Denied {
        peer: PeerCred,
        reason: &'static str,
    },
    Io(String),
}

impl std::fmt::Display for PeerAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Denied { peer, reason } => {
                write!(f, "peer uid={} gid={} denied: {reason}", peer.uid, peer.gid)
            }
            Self::Io(e) => write!(f, "peer credential I/O: {e}"),
        }
    }
}

impl std::error::Error for PeerAuthError {}

impl From<io::Error> for PeerAuthError {
    fn from(e: io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

/// Allow connections only from listed UIDs (typically the daemon's own UID).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerAllowlist {
    allowed_uids: Vec<u32>,
}

impl PeerAllowlist {
    pub fn new(allowed_uids: impl IntoIterator<Item = u32>) -> Self {
        let mut allowed_uids: Vec<u32> = allowed_uids.into_iter().collect();
        allowed_uids.sort_unstable();
        allowed_uids.dedup();
        Self { allowed_uids }
    }

    /// Default local policy: only the same effective UID as the daemon.
    pub fn same_uid(daemon_uid: u32) -> Self {
        Self::new([daemon_uid])
    }

    pub fn allowed_uids(&self) -> &[u32] {
        &self.allowed_uids
    }

    pub fn authorize(&self, peer: PeerCred) -> Result<(), PeerAuthError> {
        if self.allowed_uids.binary_search(&peer.uid).is_ok() {
            Ok(())
        } else {
            Err(PeerAuthError::Denied {
                peer,
                reason: "uid not in allowlist",
            })
        }
    }
}

/// Read peer credentials from a connected Unix stream (platform-specific).
#[cfg(unix)]
pub fn peer_cred_of(stream: &std::os::unix::net::UnixStream) -> Result<PeerCred, PeerAuthError> {
    peer_cred_raw(stream)
}

#[cfg(unix)]
fn peer_cred_raw(stream: &std::os::unix::net::UnixStream) -> Result<PeerCred, PeerAuthError> {
    use std::os::fd::AsRawFd;

    let fd = stream.as_raw_fd();
    // SAFETY: getpeereid / SO_PEERCRED on a valid connected Unix socket fd.
    // Implemented via libc without expanding forbid(unsafe_code) at crate root —
    // use a small helper module marked allow.
    cred::from_fd(fd)
}

#[cfg(unix)]
mod cred {
    #![allow(unsafe_code)]

    use super::{PeerAuthError, PeerCred};
    use std::io;

    pub fn from_fd(fd: std::os::fd::RawFd) -> Result<PeerCred, PeerAuthError> {
        from_fd_inner(fd).map_err(PeerAuthError::from)
    }

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    fn from_fd_inner(fd: std::os::fd::RawFd) -> io::Result<PeerCred> {
        let mut uid: libc::uid_t = 0;
        let mut gid: libc::gid_t = 0;
        // SAFETY: getpeereid requires a connected unix socket; uid/gid out-params.
        let rc = unsafe { libc::getpeereid(fd, &mut uid, &mut gid) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(PeerCred::new(uid as u32, gid as u32, None))
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn from_fd_inner(fd: std::os::fd::RawFd) -> io::Result<PeerCred> {
        let mut cred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of_val(&cred) as libc::socklen_t;
        // SAFETY: SO_PEERCRED on connected unix socket.
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut cred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(PeerCred::new(
            cred.uid as u32,
            cred.gid as u32,
            Some(cred.pid as u32),
        ))
    }

    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly",
        target_os = "linux",
        target_os = "android"
    )))]
    fn from_fd_inner(_fd: std::os::fd::RawFd) -> io::Result<PeerCred> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "peer credentials unsupported on this platform",
        ))
    }
}
