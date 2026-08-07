//! Release-signature transcript helpers over Capsule segment tables (F05).

use gump_crypto::{
    SegmentDigestRef, VerifyingKeyBytes, build_release_signing_transcript, verify_transcript,
};

use crate::error::{CapsuleDialectError, CapsuleDialectErrorKind};
use crate::segment::{SegmentTable, TABLE_VERSION};

/// Build the FORMATS.md §9 signing transcript for a Capsule.
pub fn release_signing_transcript(
    header_cbor: &[u8],
    table: &SegmentTable,
) -> Result<Vec<u8>, CapsuleDialectError> {
    let mut segs = [SegmentDigestRef {
        segment_type: 0,
        stored_length: 0,
        digest: [0u8; 32],
    }; 4];
    for (i, d) in table.descriptors.iter().take(4).enumerate() {
        segs[i] = SegmentDigestRef {
            segment_type: d.segment_type.as_u16(),
            stored_length: d.stored_length,
            digest: d.digest,
        };
    }
    build_release_signing_transcript(header_cbor, TABLE_VERSION, &segs)
        .map_err(|e| CapsuleDialectError::new(CapsuleDialectErrorKind::Segment, e.to_string()))
}

/// Verify an Ed25519 signature over the Capsule release transcript.
pub fn verify_release_signature(
    header_cbor: &[u8],
    table: &SegmentTable,
    public_key: &[u8; 32],
    signature: &[u8; 64],
) -> Result<(), CapsuleDialectError> {
    let transcript = release_signing_transcript(header_cbor, table)?;
    verify_transcript(&VerifyingKeyBytes(*public_key), &transcript, signature)
        .map_err(|e| CapsuleDialectError::new(CapsuleDialectErrorKind::Segment, e.to_string()))
}
