//! In-memory message fabric with explicit, deterministic faults.

use core::fmt;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::clock::InstantMillis;

use super::rng::SimRng;

/// Stable simulation peer label (not a product NodeId — those come later).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PeerId(u64);

impl PeerId {
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "peer-{}", self.0)
    }
}

/// One in-flight or delivered message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Envelope<M> {
    pub id: u64,
    pub from: PeerId,
    pub to: PeerId,
    pub payload: M,
    pub sent_at: InstantMillis,
    pub deliver_at: InstantMillis,
}

/// Fault knobs applied when a message is accepted onto the wire.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LinkFaults {
    /// Drop with probability `loss_numer / loss_denom`.
    pub loss_numer: u64,
    pub loss_denom: u64,
    /// Extra delivery delay drawn uniformly from `0..=max_delay_ms`.
    pub max_delay_ms: u64,
    /// Duplicate once with probability `dup_numer / dup_denom`.
    pub dup_numer: u64,
    pub dup_denom: u64,
}

impl Default for LinkFaults {
    fn default() -> Self {
        Self {
            loss_numer: 0,
            loss_denom: 1,
            max_delay_ms: 0,
            dup_numer: 0,
            dup_denom: 1,
        }
    }
}

impl LinkFaults {
    pub fn reliable() -> Self {
        Self::default()
    }

    pub fn with_loss(numer: u64, denom: u64) -> Self {
        Self {
            loss_numer: numer,
            loss_denom: denom.max(1),
            ..Self::default()
        }
    }

    pub fn with_max_delay_ms(max_delay_ms: u64) -> Self {
        Self {
            max_delay_ms,
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkError {
    UnknownPeer,
    PeerDown,
    Partitioned,
    Dropped,
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPeer => write!(f, "unknown peer"),
            Self::PeerDown => write!(f, "peer is down"),
            Self::Partitioned => write!(f, "peers are partitioned"),
            Self::Dropped => write!(f, "message dropped by fault model"),
        }
    }
}

impl std::error::Error for NetworkError {}

/// Delivered view handed to a running peer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Delivered<M> {
    pub id: u64,
    pub from: PeerId,
    pub payload: M,
}

#[derive(Clone, Debug)]
struct Inflight<M> {
    envelope: Envelope<M>,
}

/// Deterministic network: partitions + scheduled delivery + seeded faults.
#[derive(Clone, Debug)]
pub struct Network<M> {
    peers: BTreeSet<PeerId>,
    /// Undirected partition edges (store min,max).
    partitions: BTreeSet<(PeerId, PeerId)>,
    /// Peers that cannot send or receive (crashed).
    down: BTreeSet<PeerId>,
    faults: LinkFaults,
    next_msg_id: u64,
    /// Min-heap via BTreeMap of deliver_at → queue (stable for equal times).
    inflight: BTreeMap<u64, VecDeque<Inflight<M>>>,
    /// Dropped / rejected counts for smoke assertions.
    pub stats: NetworkStats,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NetworkStats {
    pub sent: u64,
    pub delivered: u64,
    pub dropped_loss: u64,
    pub dropped_partition: u64,
    pub dropped_down: u64,
    pub duplicated: u64,
}

impl<M> Network<M> {
    pub fn new(faults: LinkFaults) -> Self {
        Self {
            peers: BTreeSet::new(),
            partitions: BTreeSet::new(),
            down: BTreeSet::new(),
            faults,
            next_msg_id: 1,
            inflight: BTreeMap::new(),
            stats: NetworkStats::default(),
        }
    }

    pub fn faults(&self) -> LinkFaults {
        self.faults
    }

    pub fn set_faults(&mut self, faults: LinkFaults) {
        self.faults = faults;
    }

    pub fn add_peer(&mut self, peer: PeerId) {
        self.peers.insert(peer);
    }

    pub fn mark_down(&mut self, peer: PeerId) {
        self.down.insert(peer);
    }

    pub fn mark_up(&mut self, peer: PeerId) {
        self.down.remove(&peer);
    }

    pub fn is_down(&self, peer: PeerId) -> bool {
        self.down.contains(&peer)
    }

    fn edge(a: PeerId, b: PeerId) -> (PeerId, PeerId) {
        if a <= b { (a, b) } else { (b, a) }
    }

    pub fn partition(&mut self, a: PeerId, b: PeerId) {
        if a != b {
            self.partitions.insert(Self::edge(a, b));
        }
    }

    pub fn heal(&mut self, a: PeerId, b: PeerId) {
        self.partitions.remove(&Self::edge(a, b));
    }

    pub fn heal_all(&mut self) {
        self.partitions.clear();
    }

    pub fn is_partitioned(&self, a: PeerId, b: PeerId) -> bool {
        a != b && self.partitions.contains(&Self::edge(a, b))
    }

    pub fn inflight_len(&self) -> usize {
        self.inflight.values().map(|q| q.len()).sum()
    }

    /// Drop every in-flight message (e.g. after a crash policy).
    pub fn drop_inflight_involving(&mut self, peer: PeerId) {
        let mut kept = BTreeMap::new();
        for (at, queue) in std::mem::take(&mut self.inflight) {
            let mut next = VecDeque::new();
            for item in queue {
                if item.envelope.from == peer || item.envelope.to == peer {
                    self.stats.dropped_down = self.stats.dropped_down.saturating_add(1);
                } else {
                    next.push_back(item);
                }
            }
            if !next.is_empty() {
                kept.insert(at, next);
            }
        }
        self.inflight = kept;
    }

    /// Attempt to place `payload` on the wire. Returns the assigned id(s).
    pub fn send(
        &mut self,
        from: PeerId,
        to: PeerId,
        payload: M,
        now: InstantMillis,
        rng: &mut SimRng,
    ) -> Result<Vec<u64>, NetworkError>
    where
        M: Clone,
    {
        let delay = if self.faults.max_delay_ms == 0 {
            0
        } else {
            rng.gen_range(self.faults.max_delay_ms.saturating_add(1))
        };
        self.send_with_delay(from, to, payload, now, delay, rng)
    }

    /// Place `payload` with an exact delay. Loss/dup/partition still apply.
    pub fn send_with_delay(
        &mut self,
        from: PeerId,
        to: PeerId,
        payload: M,
        now: InstantMillis,
        delay_ms: u64,
        rng: &mut SimRng,
    ) -> Result<Vec<u64>, NetworkError>
    where
        M: Clone,
    {
        if !self.peers.contains(&from) || !self.peers.contains(&to) {
            return Err(NetworkError::UnknownPeer);
        }
        if self.down.contains(&from) || self.down.contains(&to) {
            self.stats.dropped_down = self.stats.dropped_down.saturating_add(1);
            return Err(NetworkError::PeerDown);
        }
        if self.is_partitioned(from, to) {
            self.stats.dropped_partition = self.stats.dropped_partition.saturating_add(1);
            return Err(NetworkError::Partitioned);
        }
        if rng.chance(self.faults.loss_numer, self.faults.loss_denom) {
            self.stats.dropped_loss = self.stats.dropped_loss.saturating_add(1);
            self.stats.sent = self.stats.sent.saturating_add(1);
            return Err(NetworkError::Dropped);
        }

        let deliver_at = InstantMillis::from_millis(now.as_millis().saturating_add(delay_ms));

        let mut ids = Vec::with_capacity(2);
        let id = self.enqueue(from, to, payload.clone(), now, deliver_at);
        ids.push(id);
        self.stats.sent = self.stats.sent.saturating_add(1);

        if rng.chance(self.faults.dup_numer, self.faults.dup_denom) {
            let delay2 = if self.faults.max_delay_ms == 0 {
                delay_ms
            } else {
                rng.gen_range(self.faults.max_delay_ms.saturating_add(1))
            };
            let deliver2 = InstantMillis::from_millis(now.as_millis().saturating_add(delay2));
            let id2 = self.enqueue(from, to, payload, now, deliver2);
            ids.push(id2);
            self.stats.duplicated = self.stats.duplicated.saturating_add(1);
            self.stats.sent = self.stats.sent.saturating_add(1);
        }

        Ok(ids)
    }

    fn enqueue(
        &mut self,
        from: PeerId,
        to: PeerId,
        payload: M,
        sent_at: InstantMillis,
        deliver_at: InstantMillis,
    ) -> u64 {
        let id = self.next_msg_id;
        self.next_msg_id = self.next_msg_id.saturating_add(1);
        let envelope = Envelope {
            id,
            from,
            to,
            payload,
            sent_at,
            deliver_at,
        };
        self.inflight
            .entry(deliver_at.as_millis())
            .or_default()
            .push_back(Inflight { envelope });
        id
    }

    /// Deliver every message with `deliver_at <= now` into per-peer inboxes.
    pub fn drain_due(&mut self, now: InstantMillis) -> BTreeMap<PeerId, Vec<Delivered<M>>> {
        let mut out: BTreeMap<PeerId, Vec<Delivered<M>>> = BTreeMap::new();
        let due_keys: Vec<u64> = self
            .inflight
            .range(..=now.as_millis())
            .map(|(k, _)| *k)
            .collect();
        for key in due_keys {
            if let Some(queue) = self.inflight.remove(&key) {
                for item in queue {
                    let env = item.envelope;
                    if self.down.contains(&env.to) || self.down.contains(&env.from) {
                        self.stats.dropped_down = self.stats.dropped_down.saturating_add(1);
                        continue;
                    }
                    if self.is_partitioned(env.from, env.to) {
                        self.stats.dropped_partition =
                            self.stats.dropped_partition.saturating_add(1);
                        continue;
                    }
                    self.stats.delivered = self.stats.delivered.saturating_add(1);
                    out.entry(env.to).or_default().push(Delivered {
                        id: env.id,
                        from: env.from,
                        payload: env.payload,
                    });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::InstantMillis;

    #[test]
    fn partition_blocks_send() {
        let mut net = Network::<&'static str>::new(LinkFaults::reliable());
        let a = PeerId::new(1);
        let b = PeerId::new(2);
        net.add_peer(a);
        net.add_peer(b);
        net.partition(a, b);
        let mut rng = SimRng::new(1);
        let err = net
            .send(a, b, "hi", InstantMillis::from_millis(0), &mut rng)
            .unwrap_err();
        assert_eq!(err, NetworkError::Partitioned);
        assert_eq!(net.inflight_len(), 0);
    }
}
