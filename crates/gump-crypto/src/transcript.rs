//! Exact byte transcripts and associated data (FORMATS.md §7–§9).

/// Prefix for protected-config AEAD / HPKE associated data.
pub const PROTECTED_AAD_PREFIX: &[u8] = b"gump.protected/1\0";

/// Prefix for the release-signature transcript.
pub const RELEASE_SIG_PREFIX: &[u8] = b"gump.release-signature/1\0";

/// HPKE info string prefix (FORMATS.md §8).
pub const HPKE_INFO_PREFIX: &[u8] = b"gump.dek/1\0";

/// One segment included in the signing transcript (types 1..=4).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SegmentDigestRef {
    pub segment_type: u16,
    pub stored_length: u64,
    pub digest: [u8; 32],
}

/// Build protected-config / HPKE associated data.
pub fn build_protected_aad(
    capsule_id: &[u8; 16],
    cluster_id: &[u8; 16],
    public_metadata_digest: &[u8; 32],
    application_archive_digest: &[u8; 32],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(PROTECTED_AAD_PREFIX.len() + 16 + 16 + 32 + 32);
    out.extend_from_slice(PROTECTED_AAD_PREFIX);
    out.extend_from_slice(capsule_id);
    out.extend_from_slice(cluster_id);
    out.extend_from_slice(public_metadata_digest);
    out.extend_from_slice(application_archive_digest);
    out
}

/// HPKE `info` = `"gump.dek/1\0" || capsule_id || cluster_id`.
pub fn hpke_info(capsule_id: &[u8; 16], cluster_id: &[u8; 16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HPKE_INFO_PREFIX.len() + 32);
    out.extend_from_slice(HPKE_INFO_PREFIX);
    out.extend_from_slice(capsule_id);
    out.extend_from_slice(cluster_id);
    out
}

/// Build the Ed25519 signing transcript (FORMATS.md §9).
///
/// `segments` MUST be types 1..=4 in ascending type order.
pub fn build_release_signing_transcript(
    header_cbor: &[u8],
    table_version: u16,
    segments: &[SegmentDigestRef; 4],
) -> Result<Vec<u8>, crate::error::CryptoError> {
    for (i, seg) in segments.iter().enumerate() {
        let expected = (i + 1) as u16;
        if seg.segment_type != expected {
            return Err(crate::error::CryptoError::new(
                crate::error::CryptoErrorKind::Encoding,
                format!(
                    "signing transcript expects segment type {expected}, got {}",
                    seg.segment_type
                ),
            ));
        }
    }
    let header_len = u32::try_from(header_cbor.len()).map_err(|_| {
        crate::error::CryptoError::new(
            crate::error::CryptoErrorKind::Length,
            "header too large for u32be length prefix",
        )
    })?;
    let mut out =
        Vec::with_capacity(RELEASE_SIG_PREFIX.len() + 4 + header_cbor.len() + 2 + 4 * (2 + 8 + 32));
    out.extend_from_slice(RELEASE_SIG_PREFIX);
    out.extend_from_slice(&header_len.to_be_bytes());
    out.extend_from_slice(header_cbor);
    out.extend_from_slice(&table_version.to_be_bytes());
    for seg in segments {
        out.extend_from_slice(&seg.segment_type.to_be_bytes());
        out.extend_from_slice(&seg.stored_length.to_be_bytes());
        out.extend_from_slice(&seg.digest);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aad_and_info_lengths() {
        let aad = build_protected_aad(&[1; 16], &[2; 16], &[3; 32], &[4; 32]);
        assert_eq!(aad.len(), PROTECTED_AAD_PREFIX.len() + 96);
        assert!(aad.starts_with(PROTECTED_AAD_PREFIX));
        let info = hpke_info(&[1; 16], &[2; 16]);
        assert_eq!(info.len(), HPKE_INFO_PREFIX.len() + 32);
    }
}
