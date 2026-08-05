//! `AcceptDeclaration` ledger with generation compare-and-swap.

use std::collections::BTreeMap;

use gump_crypto::{SignerTrustPolicy, TrustCheck, TrustError, VerifyingKeyBytes, SIGNATURE_LEN};
use gump_types::{Action, PolicyEngine, PrincipalId, WorkloadId};

use crate::declaration::normalize::{normalize_declaration, NormalizeError};
use crate::declaration::sign::{verify_declaration_signature, SignError};
use crate::declaration::types::{DeclarationDraft, NormalizedDeclaration};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeclarationError {
    Normalize(NormalizeError),
    PolicyDenied {
        decision_id: String,
    },
    Trust(TrustError),
    Signature,
    GenerationConflict {
        current: u64,
        expected: u64,
    },
    /// Same generation claimed with divergent content (INV-015).
    DivergentContent {
        generation: u64,
    },
    WorkloadMismatch,
    /// First accept must include a client-chosen workload ID (signed into intent).
    MissingWorkloadId,
}

impl std::fmt::Display for DeclarationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normalize(e) => write!(f, "{e}"),
            Self::PolicyDenied { decision_id } => {
                write!(f, "policy denied ({decision_id})")
            }
            Self::Trust(e) => write!(f, "{e}"),
            Self::Signature => write!(f, "declaration signature invalid"),
            Self::GenerationConflict { current, expected } => {
                write!(
                    f,
                    "generation conflict: current={current} expected={expected}"
                )
            }
            Self::DivergentContent { generation } => {
                write!(f, "divergent content at generation {generation}")
            }
            Self::WorkloadMismatch => write!(f, "workload_id does not match existing binding"),
            Self::MissingWorkloadId => write!(f, "workload_id required for first accept"),
        }
    }
}

impl std::error::Error for DeclarationError {}

impl From<NormalizeError> for DeclarationError {
    fn from(e: NormalizeError) -> Self {
        Self::Normalize(e)
    }
}

impl From<SignError> for DeclarationError {
    fn from(e: SignError) -> Self {
        match e {
            SignError::Normalize(n) => Self::Normalize(n),
            SignError::Crypto(_) => Self::Signature,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptResult {
    pub declaration: NormalizedDeclaration,
    pub created: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DesiredState {
    declaration: NormalizedDeclaration,
}

/// In-memory desired-state ledger keyed by `(namespace, app)`.
#[derive(Debug, Default)]
pub struct DeclarationLedger {
    by_app: BTreeMap<(String, String), DesiredState>,
}

impl DeclarationLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, namespace: &str, app: &str) -> Option<&NormalizedDeclaration> {
        self.by_app
            .get(&(namespace.to_string(), app.to_string()))
            .map(|s| &s.declaration)
    }

    /// Atomically accept a signed declaration (PROTOCOL.md §13 step 6).
    ///
    /// Signature covers intent for the proposed next generation
    /// (`expected_generation + 1`) and the bound `workload_id`.
    pub fn accept_declaration(
        &mut self,
        policy: &mut PolicyEngine,
        trust: &SignerTrustPolicy,
        principal: &PrincipalId,
        signer: &VerifyingKeyBytes,
        signature: &[u8; SIGNATURE_LEN],
        draft: DeclarationDraft,
        namespace_for_trust: &str,
        now_ms: u64,
    ) -> Result<AcceptResult, DeclarationError> {
        let auth = policy.authorize(principal, &Action::WorkloadDeploy);
        if !auth.allowed() {
            return Err(DeclarationError::PolicyDenied {
                decision_id: auth.decision_id,
            });
        }

        trust
            .check(
                signer,
                namespace_for_trust,
                now_ms,
                TrustCheck::Declaration,
                Some("deploy"),
            )
            .map_err(DeclarationError::Trust)?;

        let key = (draft.namespace.clone(), draft.app_name.clone());
        let current = self.by_app.get(&key).map(|s| s.declaration.clone());

        let (workload_id, next_gen, created) =
            resolve_generation(current.as_ref(), &draft)?;

        verify_declaration_signature(signer, &draft, workload_id, next_gen, signature)?;

        let normalized = normalize_declaration(
            &draft,
            workload_id,
            next_gen,
            auth.decision_id.clone(),
        )?;

        // Concurrent write barrier: exactly one next generation; equal-gen
        // divergent content fails closed (INV-015).
        if let Some(cur) = self.by_app.get(&key) {
            if cur.declaration.generation == normalized.generation {
                if intent_digest_of(&cur.declaration) != intent_digest_of(&normalized) {
                    return Err(DeclarationError::DivergentContent {
                        generation: cur.declaration.generation,
                    });
                }
                return Ok(AcceptResult {
                    declaration: cur.declaration.clone(),
                    created: false,
                });
            }
            if cur.declaration.generation != draft.expected_generation {
                return Err(DeclarationError::GenerationConflict {
                    current: cur.declaration.generation,
                    expected: draft.expected_generation,
                });
            }
        }

        self.by_app.insert(
            key,
            DesiredState {
                declaration: normalized.clone(),
            },
        );

        Ok(AcceptResult {
            declaration: normalized,
            created,
        })
    }
}

fn resolve_generation(
    current: Option<&NormalizedDeclaration>,
    draft: &DeclarationDraft,
) -> Result<(WorkloadId, u64, bool), DeclarationError> {
    match current {
        None => {
            if draft.expected_generation != 0 {
                return Err(DeclarationError::GenerationConflict {
                    current: 0,
                    expected: draft.expected_generation,
                });
            }
            let wid = draft.workload_id.ok_or(DeclarationError::MissingWorkloadId)?;
            Ok((wid, 1u64, true))
        }
        Some(cur) => {
            if draft.expected_generation != cur.generation {
                return Err(DeclarationError::GenerationConflict {
                    current: cur.generation,
                    expected: draft.expected_generation,
                });
            }
            if let Some(id) = draft.workload_id {
                if id != cur.workload_id {
                    return Err(DeclarationError::WorkloadMismatch);
                }
            }
            Ok((
                cur.workload_id,
                cur.generation.saturating_add(1),
                false,
            ))
        }
    }
}

/// Content identity without authorization_decision_id (stable across policy counters).
fn intent_digest_of(decl: &NormalizedDeclaration) -> [u8; 32] {
    let draft = DeclarationDraft {
        namespace: decl.namespace.as_str().to_string(),
        app_name: decl.app_name.as_str().to_string(),
        workload_id: Some(decl.workload_id),
        expected_generation: decl.generation.saturating_sub(1),
        capsule_id: decl.capsule_id,
        capsule_digest: decl.capsule_digest,
        lifecycle: decl.lifecycle.clone(),
        units: decl.units,
        operation_id: decl.operation_id,
        deployer_principal: decl.deployer_principal.clone(),
    };
    let bytes = crate::declaration::normalize::intent_bytes(
        &draft,
        decl.workload_id,
        decl.generation,
    )
    .expect("stored declaration always re-encodes");
    *blake3::hash(&bytes).as_bytes()
}
