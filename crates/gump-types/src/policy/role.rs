//! Built-in roles are action bundles only (SECURITY.md §3).

use crate::policy::action::Action;

/// Named role bundle. Enforcement still checks explicit [`Action`]s.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Role {
    /// Cluster operator: manage/join/unseal/initialize and policy manage.
    Operator,
    /// Deployer: deploy/alter/stop/forget and capsule inventory/reintroduce.
    Deployer,
    /// Reader: workload/capsule public read, telemetry subscribe, audit read.
    Reader,
    /// Agent runtime: secret deliver/resolve, connector use, hiccup use.
    Agent,
}

impl Role {
    pub fn actions(self) -> Vec<Action> {
        match self {
            Self::Operator => vec![
                Action::ClusterInitialize,
                Action::ClusterJoin,
                Action::ClusterManage,
                Action::ClusterUnseal,
                Action::PolicyRead,
                Action::PolicyManage,
                Action::CapsulePurge,
                Action::AuditRead,
            ],
            Self::Deployer => vec![
                Action::WorkloadDeploy,
                Action::WorkloadAlter,
                Action::WorkloadStop,
                Action::WorkloadForget,
                Action::WorkloadRead,
                Action::ExecutionCancel,
                Action::CapsuleInventory,
                Action::CapsuleInspectPublic,
                Action::CapsuleReintroduce,
                Action::PublicationUse {
                    provider: "*".into(),
                },
            ],
            Self::Reader => vec![
                Action::WorkloadRead,
                Action::CapsuleInventory,
                Action::CapsuleInspectPublic,
                Action::TelemetrySubscribe,
                Action::PolicyRead,
                Action::AuditRead,
            ],
            Self::Agent => vec![
                Action::SecretResolve,
                Action::SecretDeliver,
                Action::ConnectorUse { name: "*".into() },
                Action::HiccupUse,
                Action::HiccupPublish { topic: "*".into() },
                Action::HiccupListen { topic: "*".into() },
                Action::CapsuleInspectProtectedMetadata,
            ],
        }
    }
}
