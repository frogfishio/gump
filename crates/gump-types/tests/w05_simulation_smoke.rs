//! W05 exit evidence: controlled time / network / crash smoke suite.
//!
//! Authority: docs/v1/DELIVERY.md W05, docs/v1/CONFORMANCE.md §1 (`simulation`)
//! and §4 (simulator controls).

use gump_types::sim::{LinkFaults, NetworkError, PeerId, ProcessStatus, SimWorld};
use gump_types::DurationMillis;

fn transcript(world: &SimWorld<&'static str, String>) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!("t={}", world.now().as_millis()));
    for id in [PeerId::new(1), PeerId::new(2), PeerId::new(3)] {
        if let Some(p) = world.peer(id) {
            lines.push(format!(
                "{id}: status={} inc={} mem={:?}",
                p.status(),
                p.incarnation(),
                p.memory()
            ));
        }
    }
    lines.push(format!(
        "net: sent={} delivered={} loss={} part={} down={} dup={} inflight={}",
        world.network().stats.sent,
        world.network().stats.delivered,
        world.network().stats.dropped_loss,
        world.network().stats.dropped_partition,
        world.network().stats.dropped_down,
        world.network().stats.duplicated,
        world.network().inflight_len()
    ));
    lines
}

#[test]
fn controlled_time_is_deterministic() {
    let mut a = SimWorld::<(), ()>::reliable(1);
    let mut b = SimWorld::<(), ()>::reliable(1);
    a.advance(DurationMillis::from_millis(100));
    b.advance(DurationMillis::from_millis(40));
    b.advance(DurationMillis::from_millis(60));
    assert_eq!(a.now().as_millis(), b.now().as_millis());
    assert_eq!(a.now().as_millis(), 100);
}

#[test]
fn partition_drops_both_directions() {
    let mut world = SimWorld::<&'static str, ()>::reliable(9);
    let a = PeerId::new(1);
    let b = PeerId::new(2);
    world.add_peer(a);
    world.add_peer(b);
    world.partition(a, b);

    assert_eq!(
        world.send(a, b, "ab").unwrap_err(),
        NetworkError::Partitioned
    );
    assert_eq!(
        world.send(b, a, "ba").unwrap_err(),
        NetworkError::Partitioned
    );
    world.heal(a, b);
    world.send(a, b, "ok").unwrap();
    let out = world.drain();
    assert_eq!(out.delivered[&b][0].payload, "ok");
}

#[test]
fn seeded_loss_is_reproducible() {
    fn run(seed: u64) -> (u64, u64, Vec<Result<Vec<u64>, NetworkError>>) {
        let faults = LinkFaults::with_loss(1, 2); // ~50%
        let mut world = SimWorld::<u32, ()>::new(seed, faults);
        let a = PeerId::new(1);
        let b = PeerId::new(2);
        world.add_peer(a);
        world.add_peer(b);
        let mut results = Vec::new();
        for i in 0..32 {
            results.push(world.send(a, b, i));
        }
        (
            world.network().stats.dropped_loss,
            world.network().stats.sent,
            results,
        )
    }

    let (loss_a, sent_a, res_a) = run(12345);
    let (loss_b, sent_b, res_b) = run(12345);
    assert_eq!(loss_a, loss_b);
    assert_eq!(sent_a, sent_b);
    assert_eq!(res_a, res_b);
    assert!(loss_a > 0, "expected some losses with 1/2 rate");
    assert!(loss_a < 32, "expected some deliveries with 1/2 rate");
}

#[test]
fn explicit_delay_reorders_delivery_deterministically() {
    fn delivery_order() -> Vec<&'static str> {
        let mut world = SimWorld::<&'static str, ()>::reliable(1);
        let a = PeerId::new(1);
        let b = PeerId::new(2);
        world.add_peer(a);
        world.add_peer(b);
        // First send arrives later than the second → reorder.
        world.send_after(a, b, "first", 5).unwrap();
        world.send_after(a, b, "second", 1).unwrap();

        let mut got = Vec::new();
        for _ in 0..6 {
            let out = world.advance(DurationMillis::from_millis(1));
            if let Some(msgs) = out.delivered.get(&b) {
                got.extend(msgs.iter().map(|m| m.payload));
            }
        }
        got
    }

    let first = delivery_order();
    let second = delivery_order();
    assert_eq!(first, second);
    assert_eq!(first, vec!["second", "first"]);
}

#[test]
fn crash_blocks_io_restart_clears_memory() {
    let mut world = SimWorld::<&'static str, String>::reliable(3);
    let a = PeerId::new(1);
    let b = PeerId::new(2);
    world.add_peer_with(a, String::from("leader-state"));
    world.add_peer_with(b, String::from("follower-state"));

    world.send(a, b, "ping").unwrap();
    assert!(world.crash(a));
    assert_eq!(world.peer(a).unwrap().status(), ProcessStatus::Crashed);
    // In-flight involving a is dropped on crash.
    assert_eq!(world.network().inflight_len(), 0);
    assert_eq!(world.send(a, b, "nope").unwrap_err(), NetworkError::PeerDown);
    assert_eq!(world.send(b, a, "nope").unwrap_err(), NetworkError::PeerDown);

    // Survivor retains its own RAM.
    assert_eq!(world.peer(b).unwrap().memory(), "follower-state");
    assert_eq!(world.peer(a).unwrap().memory(), "leader-state");

    assert!(world.restart_empty(a));
    assert!(world.peer(a).unwrap().is_running());
    assert_eq!(world.peer(a).unwrap().memory(), "");
    assert_eq!(world.peer(a).unwrap().incarnation(), 1);

    world.send(b, a, "welcome").unwrap();
    let out = world.drain();
    assert_eq!(out.delivered[&a][0].payload, "welcome");
}

#[test]
fn three_peer_smoke_transcript_is_stable() {
    fn run() -> Vec<String> {
        let mut world = SimWorld::<&'static str, String>::reliable(42);
        let p1 = PeerId::new(1);
        let p2 = PeerId::new(2);
        let p3 = PeerId::new(3);
        world.add_peer_with(p1, String::from("A"));
        world.add_peer_with(p2, String::from("B"));
        world.add_peer_with(p3, String::from("C"));

        world.send(p1, p2, "1to2").unwrap();
        world.send(p2, p3, "2to3").unwrap();
        world.drain();

        world.partition(p1, p3);
        assert!(world.send(p1, p3, "blocked").is_err());

        world.crash(p2);
        let _ = world.send(p1, p2, "to-crashed");

        world.advance(DurationMillis::from_millis(5));
        world.restart_empty(p2);
        world.heal_all();
        world.send(p3, p2, "rejoin").unwrap();
        world.drain();

        transcript(&world)
    }

    assert_eq!(run(), run());
}
