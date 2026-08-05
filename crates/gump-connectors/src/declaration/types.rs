//! Canonical declaration types (FORMATS.md §12).

use gump_types::{CapsuleId, Label, WorkloadId};

pub const DECLARATION_SCHEMA: &str = "gump.declaration/1";

/// Provenance of an effective override field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverrideProvenance {
    Manifest,
    Deployer,
    ClusterPolicy,
}

/// Ingress/deployer-supplied draft before normalization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeclarationDraft {
    pub namespace: String,
    pub app_name: String,
    /// Omit on first deploy; present when updating an existing workload.
    pub workload_id: Option<WorkloadId>,
    /// Expected current generation; `0` means absent (first accept).
    pub expected_generation: u64,
    pub capsule_id: CapsuleId,
    pub capsule_digest: [u8; 32],
    pub lifecycle: String,
    pub units: u32,
    pub operation_id: u64,
    pub deployer_principal: String,
}

/// Canonical declaration stored in cluster memory (no Capsule bytes / secrets).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizedDeclaration {
    pub schema: &'static str,
    pub workload_id: WorkloadId,
    pub generation: u64,
    pub namespace: Label,
    pub app_name: Label,
    pub capsule_id: CapsuleId,
    pub capsule_digest: [u8; 32],
    pub lifecycle: String,
    pub units: u32,
    pub lifecycle_provenance: OverrideProvenance,
    pub deployer_principal: String,
    pub operation_id: u64,
    pub authorization_decision_id: String,
    /// BLAKE3 of the canonical signing payload (without signature).
    pub content_digest: [u8; 32],
}
