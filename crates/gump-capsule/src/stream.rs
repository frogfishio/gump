//! Streaming Capsule v0001 reader/writer for the Gump dialect.

use std::io::{Read, Write};

use capsule_lib::{Capsule, Encoding, ParseOptions, Version};
use crc32fast::Hasher as Crc32;

use crate::error::{CapsuleDialectError, CapsuleDialectErrorKind};
use crate::header::{GumpCapsuleHeader, decode_cbor_bstr, encode_cbor_bstr};
use crate::segment::{SegmentTable, SegmentType, TABLE_BYTE_LEN};

/// Default bounded read chunk for streaming verify / segment extract (STL-03).
pub const DEFAULT_STREAM_CHUNK_BYTES: usize = 64 * 1024;

/// Maximum release-signature segment retained in memory during streaming verify.
pub const MAX_SIGNATURE_SEGMENT_BYTES: usize = 256 * 1024;

/// Parsed Gump Capsule view (framing + verified segment table).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GumpCapsuleView {
    pub header: GumpCapsuleHeader,
    pub table: SegmentTable,
    /// Inner payload bytes: table prefix + five contiguous segments.
    pub inner: Vec<u8>,
    /// Exact Capsule file bytes (when available).
    pub capsule_bytes: Vec<u8>,
}

impl GumpCapsuleView {
    pub fn segment(&self, ty: SegmentType) -> &[u8] {
        self.table.segment_bytes(&self.inner, ty)
    }
}

/// Verified Capsule metadata without retaining the full inner payload (STL-03).
///
/// Large segments are hashed incrementally; only the release-signature segment
/// (bounded) is retained for trust checks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GumpCapsuleMeta {
    pub header: GumpCapsuleHeader,
    pub table: SegmentTable,
    pub signature_segment: Vec<u8>,
    /// Peak intermediate buffer used while verifying (chunk size bound).
    pub peak_buffer_bytes: usize,
}

/// Write a Gump Capsule to `writer` without requiring callers to use
/// capsule-lib's full-buffer `Capsule::to_bytes` for the dialect layer.
///
/// Internally builds encoded header/payload blocks, then serializes with
/// `capsule-lib` so CRC/prelude match the reference goldens.
pub fn write_gump_capsule<W: Write>(
    writer: &mut W,
    header: &GumpCapsuleHeader,
    segments: [&[u8]; 5],
    logical_lengths: [u64; 5],
) -> Result<GumpCapsuleView, CapsuleDialectError> {
    let parts = [
        (SegmentType::PublicMetadata, segments[0], logical_lengths[0]),
        (
            SegmentType::ApplicationArchive,
            segments[1],
            logical_lengths[1],
        ),
        (
            SegmentType::ProtectedConfig,
            segments[2],
            logical_lengths[2],
        ),
        (SegmentType::KeyEnvelope, segments[3], logical_lengths[3]),
        (
            SegmentType::ReleaseSignature,
            segments[4],
            logical_lengths[4],
        ),
    ];
    let table = SegmentTable::from_stored_parts(parts)?;
    let mut inner = table.encode();
    debug_assert_eq!(inner.len(), TABLE_BYTE_LEN as usize);
    for seg in segments {
        inner.extend_from_slice(seg);
    }
    // Structural verify before seal.
    let table = SegmentTable::parse_and_verify(&inner)?;

    let header_cbor = header.encode_cbor()?;
    let payload_cbor = encode_cbor_bstr(&inner);

    let capsule = Capsule::from_decoded(
        Version(1),
        Encoding::Cbor,
        None,
        &header_cbor,
        &payload_cbor,
    )?;
    let capsule_bytes = capsule.to_bytes()?;
    writer.write_all(&capsule_bytes)?;

    Ok(GumpCapsuleView {
        header: header.clone(),
        table,
        inner,
        capsule_bytes,
    })
}

/// Parse a full Capsule buffer into a verified Gump view.
pub fn read_gump_capsule(bytes: &[u8]) -> Result<GumpCapsuleView, CapsuleDialectError> {
    let decoded = Capsule::parse_with_options(
        bytes,
        ParseOptions {
            verify_crc: true,
            validate_encoding: true,
        },
    )?;
    if decoded.prelude.encoding != Encoding::Cbor {
        return Err(CapsuleDialectError::new(
            CapsuleDialectErrorKind::Framing,
            "gump/deployment/1 requires Encoding C",
        ));
    }
    if decoded.prelude.version.0 != 1 {
        return Err(CapsuleDialectError::new(
            CapsuleDialectErrorKind::Framing,
            format!("unsupported Capsule version {}", decoded.prelude.version.0),
        ));
    }
    let header = GumpCapsuleHeader::decode_cbor(&decoded.header_decoded)?;
    let inner = decode_cbor_bstr(&decoded.payload_decoded)?;
    let table = SegmentTable::parse_and_verify(&inner)?;
    Ok(GumpCapsuleView {
        header,
        table,
        inner,
        capsule_bytes: bytes.to_vec(),
    })
}

/// Incremental reader: pulls framing from `Read`, verifies via capsule-lib on
/// the assembled body, then exposes the Gump table without a second full copy
/// of segment digests work beyond the required inner buffer.
pub struct StreamingCapsuleReader<R> {
    inner: R,
    chunk_bytes: usize,
}

impl<R: Read> StreamingCapsuleReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            chunk_bytes: DEFAULT_STREAM_CHUNK_BYTES,
        }
    }

    pub fn with_chunk_bytes(inner: R, chunk_bytes: usize) -> Self {
        Self {
            inner,
            chunk_bytes: chunk_bytes.max(1),
        }
    }

    /// Read the entire Capsule stream into a verified view.
    ///
    /// Prefer [`Self::verify`] for production paths that must not OOM on large
    /// Capsules. This retains the full file for callers that need all segments.
    pub fn read_all(mut self) -> Result<GumpCapsuleView, CapsuleDialectError> {
        let mut buf = Vec::new();
        self.inner.read_to_end(&mut buf)?;
        read_gump_capsule(&buf)
    }

    /// Stream-verify Capsule framing, CRC, segment digests, and return metadata
    /// without retaining the application archive (STL-03 / FORMATS §3).
    pub fn verify(mut self) -> Result<GumpCapsuleMeta, CapsuleDialectError> {
        let chunk = self.chunk_bytes;
        let mut peak = 0usize;

        let mut prelude = [0u8; 24];
        self.inner.read_exact(&mut prelude)?;
        peak = peak.max(24);

        if &prelude[..7] != b"CAPSULE" {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Framing,
                "invalid Capsule magic",
            ));
        }
        let version = parse_hex_u16(&prelude[7..11]).ok_or_else(|| {
            CapsuleDialectError::new(CapsuleDialectErrorKind::Framing, "bad version field")
        })?;
        if version != 1 {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Framing,
                format!("unsupported Capsule version {version}"),
            ));
        }
        if prelude[11] != b'C' {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Framing,
                "gump/deployment/1 requires Encoding C",
            ));
        }
        let header_len = parse_hex_u16(&prelude[12..16]).ok_or_else(|| {
            CapsuleDialectError::new(CapsuleDialectErrorKind::Framing, "bad header length")
        })? as usize;
        let body_crc = parse_hex_u32(&prelude[16..24]).ok_or_else(|| {
            CapsuleDialectError::new(CapsuleDialectErrorKind::Framing, "bad body CRC field")
        })?;

        let mut header_encoded = vec![0u8; header_len];
        self.inner.read_exact(&mut header_encoded)?;
        peak = peak.max(header_len);

        let mut crc = Crc32::new();
        crc.update(&header_encoded);

        let (bstr_prefix, inner_len) = read_cbor_bstr_prefix(&mut self.inner)?;
        crc.update(&bstr_prefix);
        peak = peak.max(bstr_prefix.len());

        if inner_len < u64::from(TABLE_BYTE_LEN) {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Table,
                "inner payload shorter than segment table",
            ));
        }

        let mut table_bytes = vec![0u8; TABLE_BYTE_LEN as usize];
        self.inner.read_exact(&mut table_bytes)?;
        crc.update(&table_bytes);
        peak = peak.max(table_bytes.len());
        let table = SegmentTable::parse_table_bytes(&table_bytes)?;

        let expected_inner_end = table
            .descriptors
            .last()
            .map(|d| d.offset.saturating_add(d.stored_length))
            .unwrap_or(u64::from(TABLE_BYTE_LEN));
        if expected_inner_end != inner_len {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Table,
                format!("inner length {inner_len} != contiguous end {expected_inner_end}"),
            ));
        }

        let mut signature_segment = Vec::new();
        let mut scratch = vec![0u8; chunk];
        peak = peak.max(chunk);

        for d in &table.descriptors {
            let mut hasher = blake3::Hasher::new();
            let mut remaining = d.stored_length;
            let keep_sig = d.segment_type == SegmentType::ReleaseSignature;
            if keep_sig && d.stored_length as usize > MAX_SIGNATURE_SEGMENT_BYTES {
                return Err(CapsuleDialectError::new(
                    CapsuleDialectErrorKind::Segment,
                    "release signature segment exceeds memory bound",
                ));
            }
            while remaining > 0 {
                let n = (remaining as usize).min(scratch.len());
                self.inner.read_exact(&mut scratch[..n])?;
                crc.update(&scratch[..n]);
                hasher.update(&scratch[..n]);
                if keep_sig {
                    signature_segment.extend_from_slice(&scratch[..n]);
                }
                remaining -= n as u64;
            }
            let got = *hasher.finalize().as_bytes();
            if got != d.digest {
                return Err(CapsuleDialectError::new(
                    CapsuleDialectErrorKind::Segment,
                    format!("segment {:?} digest mismatch", d.segment_type),
                ));
            }
        }

        // No trailing Capsule bytes after the CBOR bstr.
        let mut extra = [0u8; 1];
        match self.inner.read(&mut extra)? {
            0 => {}
            _ => {
                return Err(CapsuleDialectError::new(
                    CapsuleDialectErrorKind::Framing,
                    "trailing bytes after Capsule payload",
                ));
            }
        }

        let computed = crc.finalize();
        if computed != body_crc {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Framing,
                format!("CRC mismatch: declared {body_crc:08X} computed {computed:08X}"),
            ));
        }

        let header = GumpCapsuleHeader::decode_cbor(&header_encoded)?;
        Ok(GumpCapsuleMeta {
            header,
            table,
            signature_segment,
            peak_buffer_bytes: peak,
        })
    }
}

fn parse_hex_u16(bytes: &[u8]) -> Option<u16> {
    if bytes.len() != 4 {
        return None;
    }
    let mut v: u16 = 0;
    for &b in bytes {
        let d = match b {
            b'0'..=b'9' => b - b'0',
            b'A'..=b'F' => b - b'A' + 10,
            _ => return None,
        };
        v = (v << 4) | u16::from(d);
    }
    Some(v)
}

fn parse_hex_u32(bytes: &[u8]) -> Option<u32> {
    if bytes.len() != 8 {
        return None;
    }
    let mut v: u32 = 0;
    for &b in bytes {
        let d = match b {
            b'0'..=b'9' => b - b'0',
            b'A'..=b'F' => b - b'A' + 10,
            _ => return None,
        };
        v = (v << 4) | u32::from(d);
    }
    Some(v)
}

/// Read a definite-length CBOR bstr prefix; returns (prefix_bytes, content_len).
fn read_cbor_bstr_prefix<R: Read>(r: &mut R) -> Result<(Vec<u8>, u64), CapsuleDialectError> {
    let mut first = [0u8; 1];
    r.read_exact(&mut first)?;
    let major = first[0] >> 5;
    let ai = first[0] & 0x1f;
    if major != 2 {
        return Err(CapsuleDialectError::new(
            CapsuleDialectErrorKind::Framing,
            "payload CBOR must be a byte string",
        ));
    }
    let mut prefix = vec![first[0]];
    let len = match ai {
        n @ 0..=23 => n as u64,
        24 => {
            let mut b = [0u8; 1];
            r.read_exact(&mut b)?;
            prefix.extend_from_slice(&b);
            b[0] as u64
        }
        25 => {
            let mut b = [0u8; 2];
            r.read_exact(&mut b)?;
            prefix.extend_from_slice(&b);
            u16::from_be_bytes(b) as u64
        }
        26 => {
            let mut b = [0u8; 4];
            r.read_exact(&mut b)?;
            prefix.extend_from_slice(&b);
            u32::from_be_bytes(b) as u64
        }
        27 => {
            let mut b = [0u8; 8];
            r.read_exact(&mut b)?;
            prefix.extend_from_slice(&b);
            u64::from_be_bytes(b)
        }
        _ => {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Framing,
                "non-deterministic or indefinite CBOR bstr length",
            ));
        }
    };
    Ok((prefix, len))
}

/// Incremental writer that accepts segments then finalizes framing.
pub struct StreamingCapsuleWriter {
    header: GumpCapsuleHeader,
    segments: [Vec<u8>; 5],
    logical_lengths: [u64; 5],
}

impl StreamingCapsuleWriter {
    pub fn new(header: GumpCapsuleHeader) -> Self {
        Self {
            header,
            segments: [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            logical_lengths: [0; 5],
        }
    }

    pub fn set_segment(&mut self, ty: SegmentType, bytes: impl Into<Vec<u8>>, logical_length: u64) {
        let idx = (ty.as_u16() - 1) as usize;
        self.segments[idx] = bytes.into();
        self.logical_lengths[idx] = logical_length;
    }

    pub fn finish<W: Write>(self, writer: &mut W) -> Result<GumpCapsuleView, CapsuleDialectError> {
        let refs = [
            self.segments[0].as_slice(),
            self.segments[1].as_slice(),
            self.segments[2].as_slice(),
            self.segments[3].as_slice(),
            self.segments[4].as_slice(),
        ];
        write_gump_capsule(writer, &self.header, refs, self.logical_lengths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_header() -> GumpCapsuleHeader {
        GumpCapsuleHeader {
            capsule_id: [0x11; 16],
            cluster_id: [0x22; 16],
            release_signer: "0123456789abcdef".into(),
            created_unix_ms: 42,
        }
    }

    #[test]
    fn write_read_roundtrip() {
        let mut out = Vec::new();
        let view = write_gump_capsule(
            &mut out,
            &sample_header(),
            [b"meta", b"arch", b"prot", b"keye", b"sign"],
            [4, 4, 4, 4, 4],
        )
        .unwrap();
        let again = read_gump_capsule(&out).unwrap();
        assert_eq!(again.header, view.header);
        assert_eq!(again.segment(SegmentType::ApplicationArchive), b"arch");
    }

    #[test]
    fn streaming_verify_matches_buffered_and_bounds_peak() {
        let mut out = Vec::new();
        write_gump_capsule(
            &mut out,
            &sample_header(),
            [b"meta", b"archive-payload", b"prot", b"keye", b"signatur"],
            [4, 15, 4, 4, 8],
        )
        .unwrap();
        let buffered = read_gump_capsule(&out).unwrap();
        let meta = StreamingCapsuleReader::with_chunk_bytes(out.as_slice(), 8)
            .verify()
            .unwrap();
        assert_eq!(meta.header, buffered.header);
        assert_eq!(meta.table, buffered.table);
        assert_eq!(
            meta.signature_segment,
            buffered.segment(SegmentType::ReleaseSignature)
        );
        assert!(meta.peak_buffer_bytes <= TABLE_BYTE_LEN as usize); // table dominates tiny chunk
    }
}
