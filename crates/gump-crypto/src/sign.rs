//! Ed25519 release signatures over the FORMATS.md §9 transcript.

use core::fmt;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::CryptoRng;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::{CryptoError, CryptoErrorKind};
use crate::fingerprint::ed25519_fingerprint;

pub const SIGNATURE_LEN: usize = 64;
pub const SIGNING_SUITE: &str = "Ed25519";

/// Ed25519 signing seed — zeroizing, non-Clone (SECURITY §8).
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SigningKeyBytes([u8; 32]);

impl SigningKeyBytes {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn expose(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for SigningKeyBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SigningKeyBytes(***)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifyingKeyBytes(pub [u8; 32]);

pub fn generate_signing_key<R: CryptoRng>(rng: &mut R) -> SigningKeyBytes {
    let sk = SigningKey::generate(rng);
    SigningKeyBytes::from_bytes(sk.to_bytes())
}

pub fn verifying_key(secret: &SigningKeyBytes) -> VerifyingKeyBytes {
    let sk = SigningKey::from_bytes(secret.expose());
    VerifyingKeyBytes(sk.verifying_key().to_bytes())
}

pub fn signer_fingerprint(secret: &SigningKeyBytes) -> String {
    ed25519_fingerprint(&verifying_key(secret).0)
}

/// Sign the exact transcript bytes; returns 64-byte signature.
pub fn sign_transcript(
    secret: &SigningKeyBytes,
    transcript: &[u8],
) -> Result<[u8; SIGNATURE_LEN], CryptoError> {
    let sk = SigningKey::from_bytes(secret.expose());
    let sig = sk.sign(transcript);
    Ok(sig.to_bytes())
}

/// Verify a 64-byte Ed25519 signature over the transcript.
pub fn verify_transcript(
    public: &VerifyingKeyBytes,
    transcript: &[u8],
    signature: &[u8; SIGNATURE_LEN],
) -> Result<(), CryptoError> {
    let vk = VerifyingKey::from_bytes(&public.0).map_err(|e| {
        CryptoError::new(
            CryptoErrorKind::Signature,
            format!("invalid public key: {e}"),
        )
    })?;
    let sig = Signature::from_bytes(signature);
    vk.verify(transcript, &sig)
        .map_err(|_| CryptoError::new(CryptoErrorKind::Signature, "signature verification failed"))
}
