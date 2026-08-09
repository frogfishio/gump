//! GUMP-N005: `gump server --init` forms a one-voter OpenRaft cluster.

use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gump_memory::{Command, RaftCommand};
use gump_server::accept::{AcceptStats, new_cancel_flag, run_accept_loop};
use gump_server::compose::{InitOptions, ProductRuntime};
use gump_server::framing::{read_frame, write_frame};
use gump_server::machine::{LocalCall, LocalRequest, LocalResponse, MachineOutputV1};
use gump_server::roles::RoleSet;

fn temp_sock() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("gump-n005-{nanos}.sock"))
}

#[test]
fn init_status_reports_one_voter_via_live_raft() {
    let uid = unsafe { libc::geteuid() } as u32;
    let runtime = ProductRuntime::init(InitOptions {
        roles: RoleSet::default_init(),
        peer_uid: uid,
        controller_holder: 9,
        object_store: Some(gump_connectors::RuntimeObjectStore::Memory(
            gump_connectors::FakeObjectStore::new(),
        )),
        cluster_id: None,
        signer_trust: gump_crypto::SignerTrustPolicy::new(),
    })
    .expect("init");

    let cluster = runtime
        .local_api
        .memory_cluster
        .as_ref()
        .expect("memory cluster");
    let snap = cluster.status_snapshot().unwrap();
    assert_eq!(snap.voter_count, 1);
    assert_eq!(snap.controller_holder, Some(9));
    assert!(!snap.durable_cluster_state);

    // Mutation goes through Raft client_write, not ClusterState::apply.
    let resp = cluster
        .client_write(RaftCommand::Record(Command::AdvanceTime { now_ms: 100 }))
        .unwrap();
    assert!(matches!(resp, gump_memory::RaftResponse::Applied(_)));

    let sock = temp_sock();
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock).unwrap();
    let cancel = new_cancel_flag();
    let stats = AcceptStats::new();
    let daemon = Arc::new(runtime.local_api);
    let cancel_bg = Arc::clone(&cancel);
    let accept = thread::spawn({
        let daemon = Arc::clone(&daemon);
        move || run_accept_loop(daemon, listener, cancel_bg, stats).unwrap()
    });

    thread::sleep(Duration::from_millis(40));
    let mut stream = UnixStream::connect(&sock).unwrap();
    let req = serde_json::to_vec(&LocalCall::new(LocalRequest::Status)).unwrap();
    write_frame(&mut stream, &req).unwrap();
    let frame = read_frame(&mut stream).unwrap();
    let parsed: MachineOutputV1 = serde_json::from_slice(&frame).unwrap();
    match parsed.body {
        LocalResponse::Status(s) => {
            assert_eq!(s.memory_voters, 1);
            assert_eq!(s.controller_holder, Some(9));
            assert!(s.durability_note.contains("zero failure tolerance"));
        }
        other => panic!("unexpected {other:?}"),
    }

    cancel.store(true, Ordering::SeqCst);
    thread::sleep(Duration::from_millis(40));
    accept.join().unwrap();
    if let Some(c) = &daemon.memory_cluster {
        c.shutdown().unwrap();
    }
    let _ = std::fs::remove_file(&sock);
}
