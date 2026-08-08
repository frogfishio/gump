//! Bounded resource ledger and atomic reservation (R01 / R04).

use std::collections::BTreeMap;

use gump_types::{NodeId, UnitId};

use crate::capability::NodeResources;
use crate::explain::{ExplainReason, codes};
use crate::filter::with_headroom;

/// Ceiling for retained node ledgers and reservations (bounded accounting).
pub const DEFAULT_MAX_NODES: usize = 4_096;
pub const DEFAULT_MAX_RESERVATIONS: usize = 65_536;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reservation {
    pub unit_id: UnitId,
    pub node_id: NodeId,
    pub resources: NodeResources,
    /// Capability revision pinned at commit time.
    pub capability_revision: u64,
    /// Placement fence pinned at commit time.
    pub placement_fence: u64,
}

#[derive(Clone, Debug, Default)]
struct NodeLedger {
    reserved: NodeResources,
    /// `unit_id` → reservation on this node.
    by_unit: BTreeMap<UnitId, Reservation>,
}

/// Process-local resource ledger. Atomic reserve/release under `&mut self`.
///
/// Authoritative Raft `/placements` commit is residual wiring; this ledger is
/// the scheduler-owned reservation SoT for the N011 one-server slice.
#[derive(Debug)]
pub struct ResourceLedger {
    nodes: BTreeMap<NodeId, NodeLedger>,
    /// Global index unit → node for O(1) release.
    unit_index: BTreeMap<UnitId, NodeId>,
    max_nodes: usize,
    max_reservations: usize,
}

impl Default for ResourceLedger {
    fn default() -> Self {
        Self::new()
    }
}

impl ResourceLedger {
    pub fn new() -> Self {
        Self::with_limits(DEFAULT_MAX_NODES, DEFAULT_MAX_RESERVATIONS)
    }

    pub fn with_limits(max_nodes: usize, max_reservations: usize) -> Self {
        Self {
            nodes: BTreeMap::new(),
            unit_index: BTreeMap::new(),
            max_nodes: max_nodes.max(1),
            max_reservations: max_reservations.max(1),
        }
    }

    pub fn reservation_count(&self) -> usize {
        self.unit_index.len()
    }

    pub fn reserved_on(&self, node: NodeId) -> NodeResources {
        self.nodes
            .get(&node)
            .map(|n| n.reserved)
            .unwrap_or_default()
    }

    pub fn free_on(&self, node: NodeId, allocatable: NodeResources) -> NodeResources {
        allocatable.saturating_sub(self.reserved_on(node))
    }

    pub fn get(&self, unit: UnitId) -> Option<&Reservation> {
        let node = self.unit_index.get(&unit)?;
        self.nodes.get(node)?.by_unit.get(&unit)
    }

    /// Atomically reserve headroomed resources if free capacity allows.
    ///
    /// Caller must have already hard-filtered; this still re-checks capacity
    /// and fails closed when the ledger is at ceiling.
    pub fn reserve(
        &mut self,
        unit_id: UnitId,
        node_id: NodeId,
        request: NodeResources,
        allocatable: NodeResources,
        capability_revision: u64,
        placement_fence: u64,
    ) -> Result<Reservation, ExplainReason> {
        if self.unit_index.contains_key(&unit_id) {
            return Err(ExplainReason::new(
                codes::LEDGER_FULL,
                0,
                "unit already has a reservation",
            ));
        }
        if self.unit_index.len() >= self.max_reservations {
            return Err(ExplainReason::new(
                codes::LEDGER_FULL,
                self.max_reservations as i64,
                "reservation ceiling reached",
            ));
        }
        if !self.nodes.contains_key(&node_id) && self.nodes.len() >= self.max_nodes {
            return Err(ExplainReason::new(
                codes::LEDGER_FULL,
                self.max_nodes as i64,
                "node ledger ceiling reached",
            ));
        }

        let need = with_headroom(request);
        let free = self.free_on(node_id, allocatable);
        if free.millicores < need.millicores
            || free.memory_bytes < need.memory_bytes
            || free.gpu_devices < need.gpu_devices
            || (need.ports > 0 && free.ports < need.ports)
        {
            return Err(ExplainReason::new(
                codes::MILLICORES,
                i64::from(free.millicores),
                "capacity changed before reserve commit",
            ));
        }

        let ports = if need.ports > 0 { need.ports } else { 0 };
        let commit = NodeResources {
            millicores: need.millicores,
            memory_bytes: need.memory_bytes,
            gpu_devices: need.gpu_devices,
            ports,
        };
        let reservation = Reservation {
            unit_id,
            node_id,
            resources: commit,
            capability_revision,
            placement_fence,
        };

        let entry = self.nodes.entry(node_id).or_default();
        entry.reserved = entry.reserved.saturating_add(commit);
        entry.by_unit.insert(unit_id, reservation.clone());
        self.unit_index.insert(unit_id, node_id);
        Ok(reservation)
    }

    pub fn release(&mut self, unit_id: UnitId) -> Option<Reservation> {
        let node_id = self.unit_index.remove(&unit_id)?;
        let node = self.nodes.get_mut(&node_id)?;
        let reservation = node.by_unit.remove(&unit_id)?;
        node.reserved.millicores = node
            .reserved
            .millicores
            .saturating_sub(reservation.resources.millicores);
        node.reserved.memory_bytes = node
            .reserved
            .memory_bytes
            .saturating_sub(reservation.resources.memory_bytes);
        node.reserved.gpu_devices = node
            .reserved
            .gpu_devices
            .saturating_sub(reservation.resources.gpu_devices);
        node.reserved.ports = node
            .reserved
            .ports
            .saturating_sub(reservation.resources.ports);
        if node.by_unit.is_empty() {
            self.nodes.remove(&node_id);
        }
        Some(reservation)
    }
}
