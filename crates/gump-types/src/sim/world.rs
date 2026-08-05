//! SimWorld: clock + network + processes stepped together.

use std::collections::BTreeMap;

use crate::clock::{Clock, DurationMillis, InstantMillis, ManualClock};

use super::net::{Delivered, LinkFaults, Network, NetworkError, PeerId};
use super::process::{ProcessStatus, SimProcess};
use super::rng::SimRng;

/// What happened during a single `step` / `advance`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StepOutcome<M> {
    pub delivered: BTreeMap<PeerId, Vec<Delivered<M>>>,
    pub now: InstantMillis,
}

/// Deterministic simulation world (W05 harness entry point).
#[derive(Clone, Debug)]
pub struct SimWorld<M, S = ()> {
    clock: ManualClock,
    net: Network<M>,
    peers: BTreeMap<PeerId, SimProcess<S>>,
    rng: SimRng,
}

impl<M, S: Default> SimWorld<M, S> {
    pub fn new(seed: u64, faults: LinkFaults) -> Self {
        Self {
            clock: ManualClock::new(0),
            net: Network::new(faults),
            peers: BTreeMap::new(),
            rng: SimRng::new(seed),
        }
    }

    pub fn reliable(seed: u64) -> Self {
        Self::new(seed, LinkFaults::reliable())
    }

    pub fn clock(&self) -> &ManualClock {
        &self.clock
    }

    pub fn now(&self) -> InstantMillis {
        self.clock.now()
    }

    pub fn rng_seed(&self) -> u64 {
        self.rng.seed()
    }

    pub fn network(&self) -> &Network<M> {
        &self.net
    }

    pub fn network_mut(&mut self) -> &mut Network<M> {
        &mut self.net
    }

    pub fn add_peer(&mut self, id: PeerId) {
        self.net.add_peer(id);
        self.peers.entry(id).or_insert_with(SimProcess::new);
    }

    pub fn add_peer_with(&mut self, id: PeerId, memory: S) {
        self.net.add_peer(id);
        self.peers.insert(id, SimProcess::with_memory(memory));
    }

    pub fn peer(&self, id: PeerId) -> Option<&SimProcess<S>> {
        self.peers.get(&id)
    }

    pub fn peer_mut(&mut self, id: PeerId) -> Option<&mut SimProcess<S>> {
        self.peers.get_mut(&id)
    }

    pub fn partition(&mut self, a: PeerId, b: PeerId) {
        self.net.partition(a, b);
    }

    pub fn heal(&mut self, a: PeerId, b: PeerId) {
        self.net.heal(a, b);
    }

    pub fn heal_all(&mut self) {
        self.net.heal_all();
    }

    /// Crash a peer: network marks down and drops in-flight involving it.
    pub fn crash(&mut self, id: PeerId) -> bool {
        let Some(proc) = self.peers.get_mut(&id) else {
            return false;
        };
        if !proc.is_running() {
            return false;
        }
        proc.crash();
        self.net.mark_down(id);
        self.net.drop_inflight_involving(id);
        true
    }

    /// Restart with empty memory (CONFORMANCE: member restart empty).
    pub fn restart_empty(&mut self, id: PeerId) -> bool {
        let Some(proc) = self.peers.get_mut(&id) else {
            return false;
        };
        if proc.status() != ProcessStatus::Crashed {
            return false;
        }
        proc.restart_empty();
        self.net.mark_up(id);
        true
    }

    pub fn send(&mut self, from: PeerId, to: PeerId, payload: M) -> Result<Vec<u64>, NetworkError>
    where
        M: Clone,
    {
        let now = self.clock.now();
        self.net.send(from, to, payload, now, &mut self.rng)
    }

    /// Send with an exact extra delay (bypasses random delay; still honors loss/dup/partition).
    pub fn send_after(
        &mut self,
        from: PeerId,
        to: PeerId,
        payload: M,
        delay_ms: u64,
    ) -> Result<Vec<u64>, NetworkError>
    where
        M: Clone,
    {
        let saved = self.net.faults();
        let mut forced = saved;
        forced.max_delay_ms = 0;
        self.net.set_faults(forced);
        let now = self.clock.now();
        // Temporarily schedule by advancing deliver_at via a one-shot helper.
        let result = self.net.send_with_delay(from, to, payload, now, delay_ms, &mut self.rng);
        self.net.set_faults(saved);
        result
    }

    /// Advance simulated time and deliver due messages.
    pub fn advance(&mut self, by: DurationMillis) -> StepOutcome<M> {
        self.clock.advance(by);
        self.drain()
    }

    /// Deliver whatever is already due at the current clock reading.
    pub fn drain(&mut self) -> StepOutcome<M> {
        let now = self.clock.now();
        let delivered = self.net.drain_due(now);
        StepOutcome { delivered, now }
    }

    /// Advance 1ms at a time until `predicate` or `limit_ms` elapses.
    pub fn run_until<F>(&mut self, limit_ms: u64, mut predicate: F) -> StepOutcome<M>
    where
        F: FnMut(&Self) -> bool,
        M: Clone,
    {
        let mut last = self.drain();
        if predicate(self) {
            return last;
        }
        for _ in 0..limit_ms {
            last = self.advance(DurationMillis::from_millis(1));
            if predicate(self) {
                break;
            }
        }
        last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_and_advance_delivers() {
        let mut world = SimWorld::<u32, ()>::reliable(7);
        let a = PeerId::new(1);
        let b = PeerId::new(2);
        world.add_peer(a);
        world.add_peer(b);
        world.send(a, b, 99).unwrap();
        let out = world.advance(DurationMillis::from_millis(0));
        // deliver_at == sent_at when delay is 0; advance(0) still drains due.
        let got = out.delivered.get(&b).expect("inbox");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].payload, 99);
        assert_eq!(got[0].from, a);
    }
}
