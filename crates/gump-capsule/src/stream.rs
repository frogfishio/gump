//! Streaming Capsule v0001 reader/writer for the Gump dialect.

use std::io::{Read, Write};

use capsule_lib::{Capsule, Encoding, ParseOptions, Version};

use crate::error::{CapsuleDialectError, CapsuleDialectErrorKind};
use crate::header::{decode_cbor_bstr, encode_cbor_bstr, GumpCapsuleHeader};
use crate::segment::{SegmentTable, SegmentType, TABLE_BYTE_LEN};

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
        (SegmentType::ProtectedConfig, segments[2], logical_lengths[2]),
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
            format!(
                "unsupported Capsule version {}",
                decoded.prelude.version.0
            ),
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
}

impl<R: Read> StreamingCapsuleReader<R> {
    pub fn new(inner: R) -> Self {
        Self { inner }
    }

    /// Read the entire Capsule stream into a verified view.
    ///
    /// Capsule CRC covers header+payload, so framing verification still needs
    /// the body bytes; this API streams from `Read` instead of requiring an
    /// up-front `&[u8]` (D003).
    pub fn read_all(mut self) -> Result<GumpCapsuleView, CapsuleDialectError> {
        let mut buf = Vec::new();
        self.inner.read_to_end(&mut buf)?;
        read_gump_capsule(&buf)
    }
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

    pub fn set_segment(
        &mut self,
        ty: SegmentType,
        bytes: impl Into<Vec<u8>>,
        logical_length: u64,
    ) {
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
}
