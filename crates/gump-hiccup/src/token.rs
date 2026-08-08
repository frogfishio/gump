//! Per-attempt Hiccup tokens (HICCUP.md §8). Never log or persist plaintext.

use core::fmt;

use gump_types::Secret;
use zeroize::Zeroize;

use crate::limits::{AUTH_SCHEME, TOKEN_BYTES};

/// 32-byte random token held only in authorized process memory.
pub struct HiccupToken(Secret<[u8; TOKEN_BYTES]>);

impl HiccupToken {
    pub fn generate() -> Self {
        let mut bytes = [0u8; TOKEN_BYTES];
        getrandom::fill(&mut bytes).expect("getrandom for Hiccup token");
        Self(Secret::new(bytes))
    }

    /// Test/deterministic constructor.
    pub fn from_bytes(bytes: [u8; TOKEN_BYTES]) -> Self {
        Self(Secret::new(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; TOKEN_BYTES] {
        self.0.expose()
    }

    /// `Authorization: Hiccup <hex>` value (scheme + token).
    pub fn authorization_header_value(&self) -> String {
        format!("{AUTH_SCHEME} {}", hex_encode(self.as_bytes()))
    }

    /// Constant-time validation of an `Authorization` header value.
    pub fn authorize_header(&self, header_value: &str) -> bool {
        let Some(rest) = strip_scheme(header_value) else {
            return false;
        };
        let Ok(presented) = hex_decode_exact(rest.trim(), TOKEN_BYTES) else {
            return false;
        };
        ct_eq(self.as_bytes(), &presented)
    }
}

impl fmt::Debug for HiccupToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HiccupToken(***)")
    }
}

fn strip_scheme(header_value: &str) -> Option<&str> {
    let v = header_value.trim();
    let (scheme, rest) = v.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case(AUTH_SCHEME) {
        return None;
    }
    Some(rest)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

fn hex_decode_exact(s: &str, expected: usize) -> Result<Vec<u8>, ()> {
    if s.len() != expected * 2 || !s.is_ascii() {
        return Err(());
    }
    let mut out = vec![0u8; expected];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out[i] = (hi << 4) | lo;
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8, ()> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(()),
    }
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    let mut sink = diff;
    sink.zeroize();
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrong_token_rejected() {
        let t = HiccupToken::from_bytes([7u8; 32]);
        let other = HiccupToken::from_bytes([8u8; 32]);
        assert!(t.authorize_header(&t.authorization_header_value()));
        assert!(!t.authorize_header(&other.authorization_header_value()));
        assert!(!t.authorize_header("Bearer deadbeef"));
        assert!(!t.authorize_header("Hiccup not-hex"));
    }

    #[test]
    fn debug_redacts() {
        let t = HiccupToken::from_bytes([1u8; 32]);
        assert_eq!(format!("{t:?}"), "HiccupToken(***)");
    }
}
