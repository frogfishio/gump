//! Node capability reports and resource facts (RUNTIME.md §1 / R01).

use std::collections::BTreeMap;

use gump_types::NodeId;

/// How a protection/capability is published (RUNTIME.md §1).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ProtectionLevel {
    Enforced,
    Observed,
    Unavailable,
}

impl ProtectionLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enforced => "enforced",
            Self::Observed => "observed",
            Self::Unavailable => "unavailable",
        }
    }

    /// Placement requiring enforcement accepts only [`Self::Enforced`].
    pub fn satisfies_enforcement(self) -> bool {
        matches!(self, Self::Enforced)
    }
}

/// Allocatable / reserved resource counters for one node.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NodeResources {
    pub millicores: u32,
    pub memory_bytes: u64,
    pub gpu_devices: u32,
    /// Ephemeral host ports still allocatable (0 = portless-only node).
    pub ports: u32,
}

impl NodeResources {
    pub fn saturating_sub(self, used: NodeResources) -> NodeResources {
        NodeResources {
            millicores: self.millicores.saturating_sub(used.millicores),
            memory_bytes: self.memory_bytes.saturating_sub(used.memory_bytes),
            gpu_devices: self.gpu_devices.saturating_sub(used.gpu_devices),
            ports: self.ports.saturating_sub(used.ports),
        }
    }

    pub fn saturating_add(self, other: NodeResources) -> NodeResources {
        NodeResources {
            millicores: self.millicores.saturating_add(other.millicores),
            memory_bytes: self.memory_bytes.saturating_add(other.memory_bytes),
            gpu_devices: self.gpu_devices.saturating_add(other.gpu_devices),
            ports: self.ports.saturating_add(other.ports),
        }
    }
}

/// Leased, revisioned capability report from one agent (facts only).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityReport {
    pub node_id: NodeId,
    /// Monotonic capability revision; reservations pin this (R04 stale check).
    pub revision: u64,
    /// Placement fence generation on this node.
    pub placement_fence: u64,
    pub arch: String,
    /// Execution drivers present (e.g. `native`, `script`).
    pub drivers: Vec<String>,
    /// Named capabilities → protection level.
    pub capabilities: BTreeMap<String, ProtectionLevel>,
    pub allocatable: NodeResources,
    pub drained: bool,
}

impl CapabilityReport {
    pub fn driver_supported(&self, driver: &str) -> bool {
        self.drivers.iter().any(|d| d == driver)
    }

    pub fn capability_level(&self, name: &str) -> Option<ProtectionLevel> {
        self.capabilities.get(name).copied()
    }
}

/// Declared hard requirements for one placement unit (not a full ManifestV1).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkloadRequirements {
    pub workload_id: gump_types::WorkloadId,
    pub unit_id: gump_types::UnitId,
    pub arch: String,
    /// Required driver name (`native`, `script`, …).
    pub driver: String,
    /// Capabilities that must be [`ProtectionLevel::Enforced`].
    pub required_enforced: Vec<String>,
    pub request: NodeResources,
    /// When true, node must have at least one allocatable port.
    pub requires_port: bool,
    /// Finite vs continuous — recorded for explain; not a hard filter by itself.
    pub lifecycle_finite: bool,
}
