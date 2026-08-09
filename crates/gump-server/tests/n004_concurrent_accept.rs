//! GUMP-N004: concurrent authenticated accepts + cancellable shutdown.

use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gump_server::accept::{AcceptStats, new_cancel_flag, run_accept_loop};
use gump_server::compose::{InitOptions, ProductRuntime};
use gump_server::framing::{read_frame, write_frame};
use gump_server::machine::{LocalCall, LocalRequest, MachineOutputV1};
use gump_server::roles::RoleSet;

fn temp_sock() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("gump-n004-{nanos}.sock"))
}

#[test]
fn concurrent_status_requests_and_cancel() {
    let sock = temp_sock();
    let _ = std::fs::remove_file(&sock);
    let uid = unsafe { libc::geteuid() } as u32;
    let runtime = ProductRuntime::init(InitOptions {
        roles: RoleSet::default_init(),
        peer_uid: uid,
        controller_holder: 1,
        object_store: Some(gump_connectors::RuntimeObjectStore::Memory(
            gump_connectors::FakeObjectStore::new(),
        )),
        cluster_id: None,
        signer_trust: gump_crypto::SignerTrustPolicy::new(),
    })
    .expect("compose");
    assert!(runtime.memory.enabled);
    assert!(runtime.agent.enabled);

    let listener = UnixListener::bind(&sock).expect("bind");
    let cancel = new_cancel_flag();
    let stats = AcceptStats::new();
    let daemon = Arc::new(runtime.local_api);

    let cancel_bg = Arc::clone(&cancel);
    let stats_bg = stats.clone();
    let accept = thread::spawn(move || {
        run_accept_loop(daemon, listener, cancel_bg, stats_bg).expect("accept loop")
    });

    // Wait until the listener is accepting.
    thread::sleep(Duration::from_millis(40));

    let mut clients = Vec::new();
    for _ in 0..8 {
        let path = sock.clone();
        clients.push(thread::spawn(move || {
            let mut stream = UnixStream::connect(&path).expect("connect");
            let req = serde_json::to_vec(&LocalCall::new(LocalRequest::Status)).unwrap();
            write_frame(&mut stream, &req).unwrap();
            let frame = read_frame(&mut stream).unwrap();
            let parsed: MachineOutputV1 = serde_json::from_slice(&frame).unwrap();
            assert!(matches!(
                parsed.body,
                gump_server::LocalResponse::Status(ref s) if s.memory_voters == 1
            ));
        }));
    }
    for c in clients {
        c.join().expect("client");
    }

    assert!(stats.accepted.load(Ordering::SeqCst) >= 8);

    cancel.store(true, Ordering::SeqCst);
    // Nudge the non-blocking accept sleep to notice cancel.
    thread::sleep(Duration::from_millis(40));
    accept.join().expect("accept join");
    let _ = std::fs::remove_file(&sock);
}

#[test]
fn role_parse_rejects_unknown() {
    assert!(RoleSet::from_csv("memory,not-a-role").is_err());
    let set = RoleSet::from_csv("agent,memory").unwrap();
    // NodeRole Ord → memory, agent, …
    assert_eq!(set.label(), "memory,agent");
}
