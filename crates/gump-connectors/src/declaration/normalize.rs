//! Normalize declaration drafts into canonical form.

use gump_types::{Label, LabelError, WorkloadId};

use crate::declaration::types::{
    DeclarationDraft, NormalizedDeclaration, OverrideProvenance, DECLARATION_SCHEMA,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NormalizeError {
    Label(LabelError),
    InvalidLifecycle,
    ZeroUnits,
    EmptyPrincipal,
    InvalidGeneration,
}

impl std::fmt::Display for NormalizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Label(e) => write!(f, "{e}"),
            Self::InvalidLifecycle => write!(f, "lifecycle must be finite or continuous"),
            Self::ZeroUnits => write!(f, "units must be >= 1"),
            Self::EmptyPrincipal => write!(f, "deployer principal required"),
            Self::InvalidGeneration => write!(f, "generation must be >= 1"),
        }
    }
}

impl std::error::Error for NormalizeError {}

impl From<LabelError> for NormalizeError {
    fn from(e: LabelError) -> Self {
        Self::Label(e)
    }
}

/// Normalize labels and defaults; does not allocate workload ID (accept does).
pub fn normalize_declaration(
    draft: &DeclarationDraft,
    workload_id: WorkloadId,
    generation: u64,
    authorization_decision_id: String,
) -> Result<NormalizedDeclaration, NormalizeError> {
    if generation == 0 {
        return Err(NormalizeError::InvalidGeneration);
    }
    let namespace = Label::parse(&draft.namespace)?;
    let app_name = Label::parse(&draft.app_name)?;
    if draft.deployer_principal.is_empty() {
        return Err(NormalizeError::EmptyPrincipal);
    }
    if draft.units == 0 {
        return Err(NormalizeError::ZeroUnits);
    }
    let lifecycle = draft.lifecycle.trim().to_ascii_lowercase();
    if lifecycle != "finite" && lifecycle != "continuous" {
        return Err(NormalizeError::InvalidLifecycle);
    }

    let mut decl = NormalizedDeclaration {
        schema: DECLARATION_SCHEMA,
        workload_id,
        generation,
        namespace,
        app_name,
        capsule_id: draft.capsule_id,
        capsule_digest: draft.capsule_digest,
        lifecycle,
        units: draft.units,
        lifecycle_provenance: OverrideProvenance::Deployer,
        deployer_principal: draft.deployer_principal.clone(),
        operation_id: draft.operation_id,
        authorization_decision_id,
        content_digest: [0u8; 32],
    };
    decl.content_digest = content_digest(&decl);
    Ok(decl)
}

/// Client-signed intent bytes (excludes server-assigned `authorization_decision_id`).
///
/// Includes workload ID and generation so concurrent proposers bind a specific
/// next generation (PROTOCOL.md §13 / FORMATS.md §12).
pub fn intent_bytes(
    draft: &DeclarationDraft,
    workload_id: WorkloadId,
    generation: u64,
) -> Result<Vec<u8>, NormalizeError> {
    if generation == 0 {
        return Err(NormalizeError::InvalidGeneration);
    }
    let namespace = Label::parse(&draft.namespace)?;
    let app_name = Label::parse(&draft.app_name)?;
    if draft.deployer_principal.is_empty() {
        return Err(NormalizeError::EmptyPrincipal);
    }
    if draft.units == 0 {
        return Err(NormalizeError::ZeroUnits);
    }
    let lifecycle = draft.lifecycle.trim().to_ascii_lowercase();
    if lifecycle != "finite" && lifecycle != "continuous" {
        return Err(NormalizeError::InvalidLifecycle);
    }

    let mut out = Vec::new();
    out.extend_from_slice(DECLARATION_SCHEMA.as_bytes());
    out.push(0);
    out.extend_from_slice(workload_id.to_hyphenated().as_bytes());
    out.push(0);
    out.extend_from_slice(&generation.to_be_bytes());
    out.extend_from_slice(namespace.as_str().as_bytes());
    out.push(0);
    out.extend_from_slice(app_name.as_str().as_bytes());
    out.push(0);
    out.extend_from_slice(draft.capsule_id.as_bytes());
    out.extend_from_slice(&draft.capsule_digest);
    out.extend_from_slice(lifecycle.as_bytes());
    out.push(0);
    out.extend_from_slice(&draft.units.to_be_bytes());
    out.extend_from_slice(draft.deployer_principal.as_bytes());
    out.push(0);
    out.extend_from_slice(&draft.operation_id.to_be_bytes());
    out.extend_from_slice(&draft.expected_generation.to_be_bytes());
    Ok(out)
}

/// Full stored identity bytes (includes authorization decision ID).
pub fn canonical_bytes(decl: &NormalizedDeclaration) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(decl.schema.as_bytes());
    out.push(0);
    out.extend_from_slice(decl.workload_id.to_hyphenated().as_bytes());
    out.push(0);
    out.extend_from_slice(&decl.generation.to_be_bytes());
    out.extend_from_slice(decl.namespace.as_str().as_bytes());
    out.push(0);
    out.extend_from_slice(decl.app_name.as_str().as_bytes());
    out.push(0);
    out.extend_from_slice(decl.capsule_id.as_bytes());
    out.extend_from_slice(&decl.capsule_digest);
    out.extend_from_slice(decl.lifecycle.as_bytes());
    out.push(0);
    out.extend_from_slice(&decl.units.to_be_bytes());
    out.extend_from_slice(decl.deployer_principal.as_bytes());
    out.push(0);
    out.extend_from_slice(&decl.operation_id.to_be_bytes());
    out.extend_from_slice(decl.authorization_decision_id.as_bytes());
    out
}

pub fn content_digest(decl: &NormalizedDeclaration) -> [u8; 32] {
    *blake3::hash(&canonical_bytes(decl)).as_bytes()
}
