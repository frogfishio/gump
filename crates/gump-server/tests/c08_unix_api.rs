//! C08 exit evidence: peer-auth policy and machine-output goldens.
//!
//! Authority: docs/v1/DELIVERY.md C08, DECISIONS D007.

use std::io::{Cursor, Read, Write};
use std::path::PathBuf;

use gump_server::framing::{read_frame, write_frame};
use gump_server::machine::{
    LocalRequest, LocalResponse, MachineOutputV1, sample_explain, sample_hello_response,
    sample_status,
};
use gump_server::peer::{PeerAllowlist, PeerCred};
use gump_server::serve::{LocalDaemon, handle_request, serve_connection};

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/goldens")
        .join(name)
}

fn assert_golden(name: &str, actual: &str) {
    let path = golden_path(name);
    if std::env::var_os("GUMP_WRITE_GOLDENS").is_some() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing golden {}: {e}", path.display()));
    assert_eq!(
        expected, actual,
        "golden mismatch for {name}; set GUMP_WRITE_GOLDENS=1 to regenerate"
    );
}

#[test]
fn peer_allowlist_same_uid_accepts_and_rejects() {
    let policy = PeerAllowlist::same_uid(501);
    assert!(policy.authorize(PeerCred::new(501, 20, Some(1234))).is_ok());
    let err = policy
        .authorize(PeerCred::new(502, 20, Some(99)))
        .unwrap_err();
    assert!(err.to_string().contains("denied"));
}

#[test]
fn machine_output_status_golden() {
    let out = MachineOutputV1::wrap(LocalResponse::Status(sample_status()));
    let json = out.to_canonical_json().unwrap();
    assert_golden("status_v1.json", &format!("{json}\n"));
}

#[test]
fn machine_output_hello_golden() {
    let out = MachineOutputV1::wrap(sample_hello_response());
    let json = out.to_canonical_json().unwrap();
    assert_golden("hello_v1.json", &format!("{json}\n"));
}

#[test]
fn machine_output_explain_golden() {
    let out = MachineOutputV1::wrap(sample_explain());
    let json = out.to_canonical_json().unwrap();
    assert_golden("explain_v1.json", &format!("{json}\n"));
}

#[test]
fn machine_output_unauthorized_golden() {
    let out = MachineOutputV1::wrap(gump_server::unauthorized_error());
    let json = out.to_canonical_json().unwrap();
    assert_golden("unauthorized_v1.json", &format!("{json}\n"));
}

#[test]
fn denied_peer_receives_unauthorized_machine_output() {
    let daemon = LocalDaemon::new(PeerAllowlist::same_uid(1));
    let mut buf = Cursor::new(Vec::new());
    // No request written — auth fails first and still emits a framed error.
    let body = serve_connection(&daemon, PeerCred::new(2, 2, None), &mut buf).unwrap();
    assert!(matches!(body, LocalResponse::Error(ref e) if e.code == "UNAUTHORIZED"));

    buf.set_position(0);
    let frame = read_frame(&mut buf).unwrap();
    let parsed: MachineOutputV1 = serde_json::from_slice(&frame).unwrap();
    assert_eq!(parsed.body, gump_server::unauthorized_error());
}

#[test]
fn authorized_peer_status_round_trip() {
    let mut daemon = LocalDaemon::new(PeerAllowlist::same_uid(1000));
    daemon.cluster_id = "00000000-0000-4000-8000-000000000001".into();
    daemon.controller_epoch = 3;
    daemon.controller_holder = Some(1);

    let req = serde_json::to_vec(&LocalRequest::Status).unwrap();
    let mut client_buf = Vec::new();
    write_frame(&mut client_buf, &req).unwrap();

    let mut duplex = Duplex::new(client_buf);
    let body = serve_connection(&daemon, PeerCred::new(1000, 1000, Some(1)), &mut duplex).unwrap();
    assert!(matches!(body, LocalResponse::Status(ref s) if s.controller_epoch == 3));

    let mut resp_cursor = Cursor::new(duplex.written);
    let frame = read_frame(&mut resp_cursor).unwrap();
    let parsed: MachineOutputV1 = serde_json::from_slice(&frame).unwrap();
    assert_eq!(parsed.schema, "gump.local.machine.v1");
}

#[test]
fn handle_request_hello_uses_controller_epoch() {
    let mut daemon = LocalDaemon::new(PeerAllowlist::same_uid(0));
    daemon.controller_epoch = 7;
    let resp = handle_request(&daemon, LocalRequest::Hello);
    assert_eq!(
        resp,
        LocalResponse::Hello {
            daemon: "gump-server".into(),
            controller_epoch: 7,
        }
    );
}

/// Simple in-memory duplex: reads from `to_read`, appends writes to `written`.
struct Duplex {
    to_read: Cursor<Vec<u8>>,
    written: Vec<u8>,
}

impl Duplex {
    fn new(to_read: Vec<u8>) -> Self {
        Self {
            to_read: Cursor::new(to_read),
            written: Vec::new(),
        }
    }
}

impl Read for Duplex {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.to_read.read(buf)
    }
}

impl Write for Duplex {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.written.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.written.flush()
    }
}

#[cfg(unix)]
#[test]
fn unix_socketpair_peer_cred_round_trip() {
    use std::os::unix::net::UnixStream;

    use gump_server::peer::peer_cred_of;

    let (mut client, mut server) = UnixStream::pair().unwrap();
    let cred = peer_cred_of(&server).expect("peer cred on socketpair");
    let euid = unsafe { libc::geteuid() } as u32;
    assert_eq!(cred.uid, euid);

    let daemon = LocalDaemon::new(PeerAllowlist::same_uid(euid));
    let req = serde_json::to_vec(&LocalRequest::Hello).unwrap();
    write_frame(&mut client, &req).unwrap();
    serve_connection(&daemon, cred, &mut server).unwrap();
    let frame = read_frame(&mut client).unwrap();
    let parsed: MachineOutputV1 = serde_json::from_slice(&frame).unwrap();
    assert!(matches!(parsed.body, LocalResponse::Hello { .. }));
}
