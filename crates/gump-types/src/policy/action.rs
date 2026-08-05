//! Explicit authorization actions (SECURITY.md §3).

/// Closed action vocabulary. Parameterized variants carry scope strings.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Action {
    ClusterInitialize,
    ClusterJoin,
    ClusterManage,
    ClusterUnseal,
    WorkloadDeploy,
    WorkloadRead,
    WorkloadAlter,
    WorkloadStop,
    WorkloadForget,
    ExecutionCancel,
    CapsuleInventory,
    CapsuleInspectPublic,
    CapsuleInspectProtectedMetadata,
    CapsuleReintroduce,
    CapsulePurge,
    SecretResolve,
    SecretDeliver,
    TelemetrySubscribe,
    PublicationUse { provider: String },
    ConnectorUse { name: String },
    PolicyRead,
    PolicyManage,
    AuditRead,
    HiccupUse,
    HiccupPublish { topic: String },
    HiccupListen { topic: String },
}

impl Action {
    /// Stable wire/reason name (`workload.deploy`, `hiccup.publish:foo`, …).
    pub fn as_str(&self) -> String {
        match self {
            Self::ClusterInitialize => "cluster.initialize".into(),
            Self::ClusterJoin => "cluster.join".into(),
            Self::ClusterManage => "cluster.manage".into(),
            Self::ClusterUnseal => "cluster.unseal".into(),
            Self::WorkloadDeploy => "workload.deploy".into(),
            Self::WorkloadRead => "workload.read".into(),
            Self::WorkloadAlter => "workload.alter".into(),
            Self::WorkloadStop => "workload.stop".into(),
            Self::WorkloadForget => "workload.forget".into(),
            Self::ExecutionCancel => "execution.cancel".into(),
            Self::CapsuleInventory => "capsule.inventory".into(),
            Self::CapsuleInspectPublic => "capsule.inspect_public".into(),
            Self::CapsuleInspectProtectedMetadata => "capsule.inspect_protected_metadata".into(),
            Self::CapsuleReintroduce => "capsule.reintroduce".into(),
            Self::CapsulePurge => "capsule.purge".into(),
            Self::SecretResolve => "secret.resolve".into(),
            Self::SecretDeliver => "secret.deliver".into(),
            Self::TelemetrySubscribe => "telemetry.subscribe".into(),
            Self::PublicationUse { provider } => format!("publication.use:{provider}"),
            Self::ConnectorUse { name } => format!("connector.use:{name}"),
            Self::PolicyRead => "policy.read".into(),
            Self::PolicyManage => "policy.manage".into(),
            Self::AuditRead => "audit.read".into(),
            Self::HiccupUse => "hiccup.use".into(),
            Self::HiccupPublish { topic } => format!("hiccup.publish:{topic}"),
            Self::HiccupListen { topic } => format!("hiccup.listen:{topic}"),
        }
    }

    /// Every non-parameterized action plus one sample of each parameterized form.
    pub fn coverage_matrix() -> Vec<Self> {
        vec![
            Self::ClusterInitialize,
            Self::ClusterJoin,
            Self::ClusterManage,
            Self::ClusterUnseal,
            Self::WorkloadDeploy,
            Self::WorkloadRead,
            Self::WorkloadAlter,
            Self::WorkloadStop,
            Self::WorkloadForget,
            Self::ExecutionCancel,
            Self::CapsuleInventory,
            Self::CapsuleInspectPublic,
            Self::CapsuleInspectProtectedMetadata,
            Self::CapsuleReintroduce,
            Self::CapsulePurge,
            Self::SecretResolve,
            Self::SecretDeliver,
            Self::TelemetrySubscribe,
            Self::PublicationUse {
                provider: "kismet".into(),
            },
            Self::ConnectorUse {
                name: "s3".into(),
            },
            Self::PolicyRead,
            Self::PolicyManage,
            Self::AuditRead,
            Self::HiccupUse,
            Self::HiccupPublish {
                topic: "@self".into(),
            },
            Self::HiccupListen {
                topic: "@self".into(),
            },
        ]
    }
}
