//! Domain-separated BLAKE3 fingerprints (DECISIONS D001 / SECURITY §4).

use crate::error::{CryptoError, CryptoErrorKind};

/// Domain separator for Ed25519 public-key fingerprints.
pub const ED25519_FP_DOMAIN: &[u8] = b"gump.ed25519-pub/1\0";

/// Compute lowercase `blake3:<hex>` fingerprint of an Ed25519 public key.
pub fn ed25519_fingerprint(public_key: &[u8; 32]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(ED25519_FP_DOMAIN);
    hasher.update(public_key);
    format_blake3_hex(hasher.finalize().as_bytes())
}

/// Render a 32-byte digest as `blake3:<lowercase-hex>`.
pub fn format_blake3_hex(digest: &[u8; 32]) -> String {
    let mut hex = String::with_capacity(7 + 64);
    hex.push_str("blake3:");
    for b in digest {
        hex.push_str(&format!("{b:02x}"));
    }
    hex
}

/// Parse `blake3:<64 lowercase hex>` into raw bytes.
pub fn parse_blake3_hex(s: &str) -> Result<[u8; 32], CryptoError> {
    let rest = s.strip_prefix("blake3:").ok_or_else(|| {
        CryptoError::new(
            CryptoErrorKind::Encoding,
            "fingerprint must start with blake3:",
        )
    })?;
    if rest.len() != 64 || !rest.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(CryptoError::new(
            CryptoErrorKind::Encoding,
            "fingerprint hex must be 64 lowercase/upper hex digits",
        ));
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&rest[i * 2..i * 2 + 2], 16)
            .map_err(|_| CryptoError::new(CryptoErrorKind::Encoding, "invalid fingerprint hex"))?;
    }
    Ok(out)
}
