//! Product role composition — wire facets without a parallel state path (GUMP-N004/N005).
//!
//! Memory/controller roles start a live one-voter [`gump_memory::MemoryCluster`].

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gump_agent::harden_agent_startup;
use gump_connectors::RuntimeObjectStore;
use gump_crypto::SignerTrustPolicy;
use gump_memory::{ClusterNetworkConfig, MemoryCluster};
use gump_telemetry::DEFAULT_RING_MAX_BYTES;
use gump_transport::{NodeRole, TransportLimits};
use gump_types::{ClusterId, ProcessHardenReport};

use crate::custody::ClusterCustody;
use crate::peer::PeerAllowlist;
use crate::roles::RoleSet;
use crate::runtime::RuntimeCoordinator;
use crate::serve::LocalDaemon;

/// Thin facet markers so composition stays explicit and one-way into `LocalDaemon`.
#[derive(Clone, Debug)]
pub struct MemoryFacet {
    pub enabled: bool,
    pub voters: u32,
}

#[derive(Clone, Debug)]
pub struct TransportFacet {
    pub enabled: bool,
    pub limits: TransportLimits,
}

#[derive(Clone, Debug)]
pub struct ConnectorsFacet {
    pub enabled: bool,
    /// Object-store connector type is part of the composition graph (D01).
    pub object_store: &'static str,
}

#[derive(Clone, Debug)]
pub struct SchedulerFacet {
    pub enabled: bool,
    pub crate_name: &'static str,
}

#[derive(Clone, Debug)]
pub struct AgentFacet {
    pub enabled: bool,
    pub harden: Option<ProcessHardenReport>,
}

#[derive(Clone, Debug)]
pub struct TelemetryFacet {
    pub enabled: bool,
    pub ring_capacity_bytes: usize,
}

#[derive(Clone, Debug)]
pub struct CustodyFacet {
    pub enabled: bool,
    /// Plaintext custody exists only after unseal (N008); flag records intent.
    pub sealed: bool,
}

/// Composed product runtime for one `gump server` process.
#[derive(Clone, Debug)]
pub struct ProductRuntime {
    pub cluster_id: ClusterId,
    pub roles: RoleSet,
    pub memory: MemoryFacet,
    pub transport: TransportFacet,
    pub connectors: ConnectorsFacet,
    pub scheduler: SchedulerFacet,
    pub agent: AgentFacet,
    pub telemetry: TelemetryFacet,
    pub custody: CustodyFacet,
    pub local_api: LocalDaemon,
    /// Live controller/scheduler/agent loop when all required local facets exist.
    pub execution: Option<Arc<Mutex<RuntimeCoordinator>>>,
}

#[derive(Debug)]
pub struct InitOptions {
    pub cluster_id: Option<ClusterId>,
    pub roles: RoleSet,
    pub peer_uid: u32,
    pub controller_holder: u64,
    /// Explicitly configured Capsule store. Connector/controller nodes refuse
    /// to start without one; tests may opt into the memory implementation.
    pub object_store: Option<RuntimeObjectStore>,
    pub signer_trust: SignerTrustPolicy,
}

impl Default for InitOptions {
    fn default() -> Self {
        Self {
            cluster_id: None,
            roles: RoleSet::default_init(),
            peer_uid: 0,
            controller_holder: 1,
            object_store: None,
            signer_trust: SignerTrustPolicy::new(),
        }
    }
}

impl ProductRuntime {
    /// Build facets from `--init` role selection; starts one-voter Raft when memory is on (N005).
    pub fn init(opts: InitOptions) -> Result<Self, String> {
        Self::init_with_runtime(opts, std::env::temp_dir().join("gump-state"), None)
    }

    pub fn init_with_state_root(opts: InitOptions, state_root: PathBuf) -> Result<Self, String> {
        Self::init_with_runtime(opts, state_root, None)
    }

    pub fn init_with_runtime(
        opts: InitOptions,
        state_root: PathBuf,
        cluster_network: Option<ClusterNetworkConfig>,
    ) -> Result<Self, String> {
        let cluster_id = opts.cluster_id.unwrap_or_default();
        let private_ip = cluster_network
            .as_ref()
            .map(|network| network.advertise.ip().to_string());
        let roles = opts.roles;

        let memory_on = roles.contains(NodeRole::Memory) || roles.contains(NodeRole::Controller);
        let agent_on = roles.contains(NodeRole::Agent);
        let transport_on = true; // local API always present; QUIC peers land later
        let connectors_on =
            roles.contains(NodeRole::Ingress) || roles.contains(NodeRole::Controller);
        let scheduler_on = roles.contains(NodeRole::Controller);
        let telemetry_on = agent_on || roles.contains(NodeRole::Controller);
        let custody_on = agent_on || roles.contains(NodeRole::Controller);

        let agent_harden = if agent_on {
            Some(harden_agent_startup().map_err(|e| e.to_string())?)
        } else {
            None
        };

        let mut local_api = LocalDaemon::new(PeerAllowlist::same_uid(opts.peer_uid));
        local_api.signer_trust = Arc::new(opts.signer_trust);
        local_api.cluster_id = cluster_id.to_hyphenated();
        local_api.memory_voters = if memory_on { 1 } else { 0 };
        let object_store_kind = opts.object_store.as_ref().map(RuntimeObjectStore::kind);
        if connectors_on {
            let store = opts.object_store.ok_or_else(|| {
                "connector/controller role requires an explicit Capsule object store".to_string()
            })?;
            local_api.object_store = Some(Arc::new(Mutex::new(store)));
        }
        if custody_on {
            local_api.custody = Some(Arc::new(Mutex::new(ClusterCustody::new_sealed(
                *cluster_id.as_bytes(),
            ))));
        }
        if telemetry_on {
            local_api.enable_default_telemetry(DEFAULT_RING_MAX_BYTES);
        }

        let mut memory_voters = if memory_on { 1 } else { 0 };
        if memory_on {
            // Node id 1 — single voter; controller fence committed via Raft (not direct SM).
            let cluster = Arc::new(match cluster_network {
                Some(network) => {
                    let node_id = memory_node_id(network.material.identity.node_id);
                    MemoryCluster::bootstrap_networked(node_id, opts.controller_holder, network)?
                }
                None => MemoryCluster::bootstrap_one_voter(1, opts.controller_holder)?,
            });
            let snap = cluster.status_snapshot()?;
            memory_voters = snap.voter_count;
            local_api.memory_voters = snap.voter_count;
            local_api.controller_epoch = snap.controller_epoch;
            local_api.controller_holder = snap.controller_holder;
            local_api.memory_cluster = Some(cluster);
        }

        let execution = if scheduler_on && agent_on {
            let store = local_api
                .object_store
                .as_ref()
                .ok_or_else(|| "execution requires a Capsule object store".to_string())?;
            let custody = local_api
                .custody
                .as_ref()
                .ok_or_else(|| "execution requires custody".to_string())?;
            Some(Arc::new(Mutex::new(RuntimeCoordinator::new(
                cluster_id,
                local_api
                    .memory_cluster
                    .as_ref()
                    .expect("execution has memory cluster")
                    .node_id(),
                private_ip,
                state_root,
                Arc::clone(store),
                Arc::clone(custody),
                local_api.telemetry.clone(),
            )?)))
        } else {
            None
        };
        local_api.execution = execution.clone();

        Ok(Self {
            cluster_id,
            roles,
            memory: MemoryFacet {
                enabled: memory_on,
                voters: memory_voters,
            },
            transport: TransportFacet {
                enabled: transport_on,
                limits: TransportLimits::default(),
            },
            connectors: ConnectorsFacet {
                enabled: connectors_on,
                object_store: object_store_kind.unwrap_or("disabled"),
            },
            scheduler: SchedulerFacet {
                enabled: scheduler_on,
                crate_name: "gump-scheduler",
            },
            agent: AgentFacet {
                enabled: agent_on,
                harden: agent_harden,
            },
            telemetry: TelemetryFacet {
                enabled: telemetry_on,
                ring_capacity_bytes: DEFAULT_RING_MAX_BYTES,
            },
            custody: CustodyFacet {
                enabled: custody_on,
                sealed: local_api
                    .custody
                    .as_ref()
                    .and_then(|c| c.lock().ok().map(|g| g.is_sealed()))
                    .unwrap_or(true),
            },
            local_api,
            execution,
        })
    }

    pub fn status_line(&self) -> String {
        let sealed = self
            .local_api
            .custody
            .as_ref()
            .and_then(|c| c.lock().ok().map(|g| g.is_sealed()))
            .unwrap_or(self.custody.sealed);
        format!(
            "cluster={} roles={} memory_voters={} agent={} scheduler={} connectors={} telemetry={} custody_sealed={}",
            self.cluster_id.to_hyphenated(),
            self.roles.label(),
            self.memory.voters,
            self.agent.enabled,
            self.scheduler.enabled,
            self.connectors.enabled,
            self.telemetry.enabled,
            sealed
        )
    }
}

fn memory_node_id(node_id: gump_types::NodeId) -> u64 {
    u64::from_be_bytes(
        node_id.as_bytes()[8..16]
            .try_into()
            .expect("NodeId suffix is 8 bytes"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_default_roles_wires_memory_agent_controller() {
        let rt = ProductRuntime::init(InitOptions {
            peer_uid: 501,
            object_store: Some(RuntimeObjectStore::Memory(
                gump_connectors::FakeObjectStore::new(),
            )),
            ..InitOptions::default()
        })
        .expect("init");
        assert!(rt.memory.enabled);
        assert_eq!(rt.memory.voters, 1);
        assert!(rt.agent.enabled);
        assert!(rt.scheduler.enabled);
        assert!(rt.local_api.controller_holder.is_some());
        assert!(rt.local_api.memory_cluster.is_some());
        assert_eq!(rt.local_api.allowlist, PeerAllowlist::same_uid(501));
        assert_eq!(rt.connectors.object_store, "memory-test");
        assert!(rt.local_api.object_store.is_some());
        assert_eq!(rt.scheduler.crate_name, "gump-scheduler");
        let snap = rt
            .local_api
            .memory_cluster
            .as_ref()
            .unwrap()
            .status_snapshot()
            .unwrap();
        assert_eq!(snap.voter_count, 1);
        assert!(!snap.durable_cluster_state);
        assert!(rt.local_api.custody.is_some());
        assert!(
            rt.local_api
                .custody
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .is_sealed()
        );
    }

    #[test]
    fn agent_only_skips_controller_bootstrap() {
        let roles = RoleSet::from_csv("agent").unwrap();
        let rt = ProductRuntime::init(InitOptions {
            cluster_id: None,
            roles,
            peer_uid: 1,
            controller_holder: 1,
            object_store: None,
            signer_trust: SignerTrustPolicy::new(),
        })
        .unwrap();
        assert!(rt.agent.enabled);
        assert!(!rt.scheduler.enabled);
        assert_eq!(rt.local_api.controller_epoch, 0);
        assert!(rt.local_api.controller_holder.is_none());
    }
}
