//! XChaCha20-Poly1305 seal for protected-config plaintext (FORMATS.md §7).

use core::fmt;

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use gump_types::Secret;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::{CryptoError, CryptoErrorKind};

pub const DEK_LEN: usize = 32;
pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;

/// Zeroizing DEK — public APIs return this instead of bare `[u8; 32]` (SECURITY §8).
///
/// Not `Clone`: callers must borrow via [`Dek::expose`].
#[derive(Eq, PartialEq, Zeroize, ZeroizeOnDrop)]
pub struct Dek([u8; DEK_LEN]);

impl Dek {
    pub const fn new(bytes: [u8; DEK_LEN]) -> Self {
        Self(bytes)
    }

    pub fn expose(&self) -> &[u8; DEK_LEN] {
        &self.0
    }
}

impl fmt::Debug for Dek {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Dek(***)")
    }
}

impl AsRef<[u8; DEK_LEN]> for Dek {
    fn as_ref(&self) -> &[u8; DEK_LEN] {
        &self.0
    }
}

/// Protected-config plaintext after AEAD open — zeroizing, non-Clone, redacted Debug.
pub type ProtectedPlaintext = Secret<Vec<u8>>;

/// Encrypt plaintext; returns ciphertext || 16-byte tag (nonce is stored in the key envelope).
pub fn seal_protected(
    dek: &[u8; DEK_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let cipher = XChaCha20Poly1305::new(&Key::from(*dek));
    let nonce = XNonce::from(*nonce);
    cipher
        .encrypt(
            &nonce,
            chacha20poly1305::aead::Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| CryptoError::new(CryptoErrorKind::Aead, "seal failed"))
}

/// Decrypt ciphertext||tag under AAD into a zeroizing container.
pub fn open_protected(
    dek: &[u8; DEK_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
) -> Result<ProtectedPlaintext, CryptoError> {
    if ciphertext_and_tag.len() < TAG_LEN {
        return Err(CryptoError::new(
            CryptoErrorKind::Length,
            "protected ciphertext shorter than tag",
        ));
    }
    let cipher = XChaCha20Poly1305::new(&Key::from(*dek));
    let nonce = XNonce::from(*nonce);
    let plaintext = cipher
        .decrypt(
            &nonce,
            chacha20poly1305::aead::Payload {
                msg: ciphertext_and_tag,
                aad,
            },
        )
        .map_err(|_| CryptoError::new(CryptoErrorKind::Aead, "open failed"))?;
    Ok(Secret::new(plaintext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dek_and_protected_debug_redact() {
        let dek = Dek::new([0x42; DEK_LEN]);
        assert_eq!(format!("{dek:?}"), "Dek(***)");
        assert!(!format!("{dek:?}").contains("42"));
        let pt = Secret::new(b"hunter2-plaintext".to_vec());
        assert_eq!(format!("{pt:?}"), "Secret(***)");
        assert!(!format!("{pt:?}").contains("hunter2"));
    }
}
