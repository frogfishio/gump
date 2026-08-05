//! Ed25519 signatures over declaration intent bytes.

use gump_crypto::{
    sign_transcript, verify_transcript, SigningKeyBytes, VerifyingKeyBytes, SIGNATURE_LEN,
};
use gump_types::WorkloadId;

use crate::declaration::normalize::{intent_bytes, NormalizeError};
use crate::declaration::types::DeclarationDraft;

/// Domain separator for declaration signatures.
pub const DECLARATION_SIG_DOMAIN: &[u8] = b"gump.declaration.sig/1\0";

fn signing_payload(
    draft: &DeclarationDraft,
    workload_id: WorkloadId,
    generation: u64,
) -> Result<Vec<u8>, NormalizeError> {
    let mut out = DECLARATION_SIG_DOMAIN.to_vec();
    out.extend_from_slice(&intent_bytes(draft, workload_id, generation)?);
    Ok(out)
}

/// Sign client intent for a proposed next generation (server fills decision ID).
pub fn sign_declaration(
    secret: &SigningKeyBytes,
    draft: &DeclarationDraft,
    workload_id: WorkloadId,
    generation: u64,
) -> Result<[u8; SIGNATURE_LEN], SignError> {
    let payload = signing_payload(draft, workload_id, generation).map_err(SignError::Normalize)?;
    sign_transcript(secret, &payload).map_err(SignError::Crypto)
}

pub fn verify_declaration_signature(
    public: &VerifyingKeyBytes,
    draft: &DeclarationDraft,
    workload_id: WorkloadId,
    generation: u64,
    signature: &[u8; SIGNATURE_LEN],
) -> Result<(), SignError> {
    let payload = signing_payload(draft, workload_id, generation).map_err(SignError::Normalize)?;
    verify_transcript(public, &payload, signature).map_err(SignError::Crypto)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SignError {
    Normalize(NormalizeError),
    Crypto(gump_crypto::CryptoError),
}

impl std::fmt::Display for SignError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normalize(e) => write!(f, "{e}"),
            Self::Crypto(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SignError {}
