//! Deterministic CBOR header for `gump/deployment/1` (FORMATS.md §2).

use crate::error::{CapsuleDialectError, CapsuleDialectErrorKind};

pub const DIALECT: &str = "gump/deployment/1";
pub const PAYLOAD_LAYOUT: &str = "gump-segments/1";

/// Decoded Capsule header map (Encoding C, deterministic CBOR).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GumpCapsuleHeader {
    pub capsule_id: [u8; 16],
    pub cluster_id: [u8; 16],
    pub release_signer: String,
    pub created_unix_ms: i64,
}

impl GumpCapsuleHeader {
    /// Encode as a definite CBOR map with keys sorted by encoded form (RFC 8949).
    ///
    /// Encoded-key order for these text keys:
    /// `dialect`, `capsule_id`, `cluster_id`, `payload_layout`, `release_signer`,
    /// `created_unix_ms`.
    pub fn encode_cbor(&self) -> Result<Vec<u8>, CapsuleDialectError> {
        if self.release_signer.is_empty() || self.release_signer.len() > 256 {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Header,
                "release_signer length out of range",
            ));
        }
        if !self
            .release_signer
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Header,
                "release_signer must be lowercase hex fingerprint",
            ));
        }

        let mut out = Vec::with_capacity(128);
        // map(6)
        out.push(0xa6);
        encode_text(&mut out, "dialect");
        encode_text(&mut out, DIALECT);
        encode_text(&mut out, "capsule_id");
        encode_bstr16(&mut out, &self.capsule_id);
        encode_text(&mut out, "cluster_id");
        encode_bstr16(&mut out, &self.cluster_id);
        encode_text(&mut out, "payload_layout");
        encode_text(&mut out, PAYLOAD_LAYOUT);
        encode_text(&mut out, "release_signer");
        encode_text(&mut out, &self.release_signer);
        encode_text(&mut out, "created_unix_ms");
        encode_int(&mut out, self.created_unix_ms);
        Ok(out)
    }

    pub fn decode_cbor(bytes: &[u8]) -> Result<Self, CapsuleDialectError> {
        let mut cur = Cursor::new(bytes);
        let len = cur.read_map_len()?;
        if len != 6 {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Header,
                format!("header map must have 6 entries, got {len}"),
            ));
        }

        let mut dialect: Option<String> = None;
        let mut capsule_id: Option<[u8; 16]> = None;
        let mut cluster_id: Option<[u8; 16]> = None;
        let mut payload_layout: Option<String> = None;
        let mut release_signer: Option<String> = None;
        let mut created_unix_ms: Option<i64> = None;
        let mut last_key_enc: Vec<u8> = Vec::new();

        for _ in 0..len {
            let key_start = cur.pos;
            let key = cur.read_text()?;
            let key_enc = bytes[key_start..cur.pos].to_vec();
            if !last_key_enc.is_empty() && key_enc <= last_key_enc {
                return Err(CapsuleDialectError::new(
                    CapsuleDialectErrorKind::Header,
                    "header map keys not in deterministic order",
                ));
            }
            last_key_enc = key_enc;

            match key.as_str() {
                "dialect" => {
                    dialect = Some(cur.read_text()?);
                }
                "capsule_id" => {
                    capsule_id = Some(cur.read_bstr_exact(16)?);
                }
                "cluster_id" => {
                    cluster_id = Some(cur.read_bstr_exact(16)?);
                }
                "payload_layout" => {
                    payload_layout = Some(cur.read_text()?);
                }
                "release_signer" => {
                    release_signer = Some(cur.read_text()?);
                }
                "created_unix_ms" => {
                    created_unix_ms = Some(cur.read_int()?);
                }
                other => {
                    return Err(CapsuleDialectError::new(
                        CapsuleDialectErrorKind::Header,
                        format!("unknown header key {other:?}"),
                    ));
                }
            }
        }
        if cur.pos != bytes.len() {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Header,
                "trailing bytes after header map",
            ));
        }

        let dialect = dialect.ok_or_else(|| missing("dialect"))?;
        if dialect != DIALECT {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Header,
                format!("unsupported dialect {dialect:?}"),
            ));
        }
        let payload_layout = payload_layout.ok_or_else(|| missing("payload_layout"))?;
        if payload_layout != PAYLOAD_LAYOUT {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Header,
                format!("unsupported payload_layout {payload_layout:?}"),
            ));
        }

        Ok(Self {
            capsule_id: capsule_id.ok_or_else(|| missing("capsule_id"))?,
            cluster_id: cluster_id.ok_or_else(|| missing("cluster_id"))?,
            release_signer: release_signer.ok_or_else(|| missing("release_signer"))?,
            created_unix_ms: created_unix_ms.ok_or_else(|| missing("created_unix_ms"))?,
        })
    }
}

fn missing(field: &str) -> CapsuleDialectError {
    CapsuleDialectError::new(
        CapsuleDialectErrorKind::Header,
        format!("missing header field {field}"),
    )
}

fn encode_text(out: &mut Vec<u8>, s: &str) {
    encode_len(out, 3, s.len());
    out.extend_from_slice(s.as_bytes());
}

fn encode_bstr16(out: &mut Vec<u8>, bytes: &[u8; 16]) {
    encode_len(out, 2, 16);
    out.extend_from_slice(bytes);
}

fn encode_int(out: &mut Vec<u8>, v: i64) {
    if v >= 0 {
        encode_len(out, 0, v as usize);
    } else {
        let n = (-1 - v) as usize;
        encode_len(out, 1, n);
    }
}

fn encode_len(out: &mut Vec<u8>, major: u8, len: usize) {
    let mt = major << 5;
    if len < 24 {
        out.push(mt | (len as u8));
    } else if len <= u8::MAX as usize {
        out.push(mt | 24);
        out.push(len as u8);
    } else if len <= u16::MAX as usize {
        out.push(mt | 25);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else if len <= u32::MAX as usize {
        out.push(mt | 26);
        out.extend_from_slice(&(len as u32).to_be_bytes());
    } else {
        out.push(mt | 27);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
}

/// Encode a CBOR byte string wrapping `inner`.
pub(crate) fn encode_cbor_bstr(inner: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(9 + inner.len());
    encode_len(&mut out, 2, inner.len());
    out.extend_from_slice(inner);
    out
}

/// CBOR bstr header for a payload of `inner_len` bytes (no content).
pub(crate) fn encode_cbor_bstr_prefix(inner_len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(9);
    encode_len(&mut out, 2, inner_len);
    out
}

/// Decode a single CBOR byte string; reject trailing items.
pub(crate) fn decode_cbor_bstr(bytes: &[u8]) -> Result<Vec<u8>, CapsuleDialectError> {
    let mut cur = Cursor::new(bytes);
    let inner = cur.read_bstr()?;
    if cur.pos != bytes.len() {
        return Err(CapsuleDialectError::new(
            CapsuleDialectErrorKind::Framing,
            "payload CBOR must be exactly one byte string",
        ));
    }
    Ok(inner)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], CapsuleDialectError> {
        if self.pos + n > self.bytes.len() {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Header,
                "truncated CBOR",
            ));
        }
        let s = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn read_initial(&mut self) -> Result<(u8, u64), CapsuleDialectError> {
        let first = self.take(1)?[0];
        let major = first >> 5;
        let ai = first & 0x1f;
        let val = match ai {
            n @ 0..=23 => n as u64,
            24 => self.take(1)?[0] as u64,
            25 => {
                let b = self.take(2)?;
                u16::from_be_bytes([b[0], b[1]]) as u64
            }
            26 => {
                let b = self.take(4)?;
                u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64
            }
            27 => {
                let b = self.take(8)?;
                u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
            }
            _ => {
                return Err(CapsuleDialectError::new(
                    CapsuleDialectErrorKind::Header,
                    "non-deterministic or indefinite CBOR length",
                ));
            }
        };
        Ok((major, val))
    }

    fn read_map_len(&mut self) -> Result<usize, CapsuleDialectError> {
        let (major, val) = self.read_initial()?;
        if major != 5 {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Header,
                "expected CBOR map",
            ));
        }
        Ok(val as usize)
    }

    fn read_text(&mut self) -> Result<String, CapsuleDialectError> {
        let (major, val) = self.read_initial()?;
        if major != 3 {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Header,
                "expected CBOR text",
            ));
        }
        let s = self.take(val as usize)?;
        String::from_utf8(s.to_vec()).map_err(|_| {
            CapsuleDialectError::new(CapsuleDialectErrorKind::Header, "header text not UTF-8")
        })
    }

    fn read_bstr(&mut self) -> Result<Vec<u8>, CapsuleDialectError> {
        let (major, val) = self.read_initial()?;
        if major != 2 {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Framing,
                "expected CBOR byte string",
            ));
        }
        Ok(self.take(val as usize)?.to_vec())
    }

    fn read_bstr_exact(&mut self, n: usize) -> Result<[u8; 16], CapsuleDialectError> {
        let bytes = self.read_bstr()?;
        if bytes.len() != n || n != 16 {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Header,
                format!("expected bstr length {n}, got {}", bytes.len()),
            ));
        }
        let mut out = [0u8; 16];
        out.copy_from_slice(&bytes);
        Ok(out)
    }

    fn read_int(&mut self) -> Result<i64, CapsuleDialectError> {
        let (major, val) = self.read_initial()?;
        match major {
            0 => i64::try_from(val).map_err(|_| {
                CapsuleDialectError::new(CapsuleDialectErrorKind::Header, "integer overflow")
            }),
            1 => {
                let n = i64::try_from(val).map_err(|_| {
                    CapsuleDialectError::new(CapsuleDialectErrorKind::Header, "integer overflow")
                })?;
                Ok(-1 - n)
            }
            _ => Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Header,
                "expected CBOR integer",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip_deterministic() {
        let h = GumpCapsuleHeader {
            capsule_id: [1; 16],
            cluster_id: [2; 16],
            release_signer: "aabbccddeeff0011".into(),
            created_unix_ms: 1_700_000_000_000,
        };
        let a = h.encode_cbor().unwrap();
        let b = h.encode_cbor().unwrap();
        assert_eq!(a, b);
        assert_eq!(GumpCapsuleHeader::decode_cbor(&a).unwrap(), h);
    }
}
