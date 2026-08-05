//! XChaCha20-Poly1305 seal for protected-config plaintext (FORMATS.md §7).

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::{CryptoError, CryptoErrorKind};

pub const DEK_LEN: usize = 32;
pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;

/// Zeroizing DEK wrapper.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Dek(pub [u8; DEK_LEN]);

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

/// Decrypt ciphertext||tag under AAD.
pub fn open_protected(
    dek: &[u8; DEK_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if ciphertext_and_tag.len() < TAG_LEN {
        return Err(CryptoError::new(
            CryptoErrorKind::Length,
            "protected ciphertext shorter than tag",
        ));
    }
    let cipher = XChaCha20Poly1305::new(&Key::from(*dek));
    let nonce = XNonce::from(*nonce);
    cipher
        .decrypt(
            &nonce,
            chacha20poly1305::aead::Payload {
                msg: ciphertext_and_tag,
                aad,
            },
        )
        .map_err(|_| CryptoError::new(CryptoErrorKind::Aead, "open failed"))
}
