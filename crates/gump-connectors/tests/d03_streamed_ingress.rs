//! D03 exit evidence: peak-memory streaming ingest + corrupt-input suite.
//!
//! Authority: docs/v1/DELIVERY.md D03, SYSTEM_DESIGN §13.1.1, SECURITY §4.

use std::collections::BTreeSet;
use std::io::Cursor;

use gump_capsule::{verify_release_signature, write_gump_capsule, GumpCapsuleHeader};
use gump_connectors::{
    FakeObjectStore, IngressError, IngressLimits, ObjectStore, StreamedIngress,
};
use gump_crypto::{
    build_release_signing_transcript, ed25519_fingerprint, generate_signing_key, sign_transcript,
    verifying_key, SegmentDigestRef, SignerEnrollment, SignerTrustPolicy, VerifyingKeyBytes,
};
use gump_types::{CapsuleId, ClusterId};
use rand_core::{TryCryptoRng, TryRng};

struct SeedRng {
    state: u64,
}

impl SeedRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }
}

impl TryRng for SeedRng {
    type Error = core::convert::Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
        Ok((self.state >> 32) as u32)
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        let a = self.try_next_u32()? as u64;
        let b = self.try_next_u32()? as u64;
        Ok((a << 32) | b)
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Self::Error> {
        for chunk in dest.chunks_mut(4) {
            let n = self.try_next_u32()?.to_le_bytes();
            for (i, b) in chunk.iter_mut().enumerate() {
                *b = n[i];
            }
        }
        Ok(())
    }
}

impl TryCryptoRng for SeedRng {}

fn v7(seed: u8) -> [u8; 16] {
    let mut b = [seed; 16];
    b[6] = (b[6] & 0x0f) | 0x70;
    b[8] = (b[8] & 0x3f) | 0x80;
    b
}

fn ids() -> (ClusterId, CapsuleId) {
    (
        ClusterId::from_bytes(v7(0x51)).unwrap(),
        CapsuleId::from_bytes(v7(0x52)).unwrap(),
    )
}

struct SealedFixture {
    bytes: Vec<u8>,
    verifying: VerifyingKeyBytes,
    cluster: ClusterId,
    capsule: CapsuleId,
}

fn build_sealed(seed: u64) -> SealedFixture {
    let (cluster, capsule) = ids();
    let mut rng = SeedRng::new(seed);
    let signing = generate_signing_key(&mut rng);
    let verifying = verifying_key(&signing);
    let fp = ed25519_fingerprint(&verifying.0);
    let release_signer = fp.strip_prefix("blake3:").unwrap().to_string();

    let public_metadata = b"meta".as_slice();
    let archive = b"archive-bytes".as_slice();
    let protected = b"protected".as_slice();
    let key_envelope = b"envelope".as_slice();

    let header = GumpCapsuleHeader {
        capsule_id: *capsule.as_bytes(),
        cluster_id: *cluster.as_bytes(),
        release_signer,
        created_unix_ms: 0,
    };
    let header_cbor = header.encode_cbor().unwrap();
    let logical = [
        public_metadata.len() as u64,
        archive.len() as u64,
        protected.len() as u64,
        key_envelope.len() as u64,
        0,
    ];
    let placeholder = [0u8; 96];
    let mut buf = Vec::new();
    let provisional = write_gump_capsule(
        &mut buf,
        &header,
        [
            public_metadata,
            archive,
            protected,
            key_envelope,
            placeholder.as_slice(),
        ],
        logical,
    )
    .unwrap();

    let segs = [
        SegmentDigestRef {
            segment_type: 1,
            stored_length: provisional.table.descriptors[0].stored_length,
            digest: provisional.table.descriptors[0].digest,
        },
        SegmentDigestRef {
            segment_type: 2,
            stored_length: provisional.table.descriptors[1].stored_length,
            digest: provisional.table.descriptors[1].digest,
        },
        SegmentDigestRef {
            segment_type: 3,
            stored_length: provisional.table.descriptors[2].stored_length,
            digest: provisional.table.descriptors[2].digest,
        },
        SegmentDigestRef {
            segment_type: 4,
            stored_length: provisional.table.descriptors[3].stored_length,
            digest: provisional.table.descriptors[3].digest,
        },
    ];
    let transcript = build_release_signing_transcript(&header_cbor, 1, &segs).unwrap();
    let signature = sign_transcript(&signing, &transcript).unwrap();
    let mut sig_seg = Vec::with_capacity(96);
    sig_seg.extend_from_slice(&verifying.0);
    sig_seg.extend_from_slice(&signature);

    let mut sealed = Vec::new();
    let view = write_gump_capsule(
        &mut sealed,
        &header,
        [
            public_metadata,
            archive,
            protected,
            key_envelope,
            sig_seg.as_slice(),
        ],
        logical,
    )
    .unwrap();
    verify_release_signature(&header_cbor, &view.table, &verifying.0, &signature).unwrap();

    SealedFixture {
        bytes: sealed,
        verifying,
        cluster,
        capsule,
    }
}

fn enroll(trust: &mut SignerTrustPolicy, vk: VerifyingKeyBytes, ns: &str) {
    trust
        .enroll(SignerEnrollment {
            public_key: vk,
            namespaces: BTreeSet::from([ns.into()]),
            expires_at_ms: None,
            capabilities: BTreeSet::new(),
        })
        .unwrap();
}

#[test]
fn happy_path_streams_and_publishes() {
    let fix = build_sealed(1);
    let mut trust = SignerTrustPolicy::new();
    enroll(&mut trust, fix.verifying, "prod");
    let mut store = FakeObjectStore::new();
    let ingress = StreamedIngress::new(IngressLimits {
        max_capsule_bytes: 10 * 1024 * 1024,
        max_chunk_bytes: 64,
    });
    let mut reader = Cursor::new(fix.bytes.clone());
    let receipt = ingress
        .accept_known_length(
            &mut store,
            &trust,
            fix.cluster,
            fix.capsule,
            "prod",
            0,
            fix.bytes.len() as u64,
            &mut reader,
        )
        .unwrap();
    assert_eq!(receipt.stats.bytes_received, fix.bytes.len() as u64);
    assert!(receipt.stats.peak_buffer_bytes <= 64);
    assert_eq!(
        store.get(&receipt.evidence.key, None).unwrap(),
        fix.bytes
    );
}

#[test]
fn peak_memory_bounded_for_large_body() {
    let fix = build_sealed(2);
    // Pad by repeating? Can't pad sealed capsule. Stream the sealed bytes with tiny chunks.
    let mut trust = SignerTrustPolicy::new();
    enroll(&mut trust, fix.verifying, "prod");
    let mut store = FakeObjectStore::new();
    let chunk = 16usize;
    let ingress = StreamedIngress::new(IngressLimits {
        max_capsule_bytes: 10 * 1024 * 1024,
        max_chunk_bytes: chunk,
    });
    let mut reader = Cursor::new(fix.bytes.clone());
    let receipt = ingress
        .accept_known_length(
            &mut store,
            &trust,
            fix.cluster,
            fix.capsule,
            "prod",
            0,
            fix.bytes.len() as u64,
            &mut reader,
        )
        .unwrap();
    assert!(
        receipt.stats.peak_buffer_bytes <= chunk,
        "peak {} exceeded chunk {}",
        receipt.stats.peak_buffer_bytes,
        chunk
    );
}

#[test]
fn corrupt_byte_fails_structural_or_signature() {
    let fix = build_sealed(3);
    let mut trust = SignerTrustPolicy::new();
    enroll(&mut trust, fix.verifying, "prod");
    let mut store = FakeObjectStore::new();
    let mut bad = fix.bytes.clone();
    let mid = bad.len() / 2;
    bad[mid] ^= 0xff;
    let ingress = StreamedIngress::default();
    let mut reader = Cursor::new(bad.clone());
    let err = ingress
        .accept_known_length(
            &mut store,
            &trust,
            fix.cluster,
            fix.capsule,
            "prod",
            0,
            bad.len() as u64,
            &mut reader,
        )
        .unwrap_err();
    assert!(
        matches!(err, IngressError::Capsule(_) | IngressError::Signature),
        "unexpected {err}"
    );
}

#[test]
fn truncated_body_rejected() {
    let fix = build_sealed(4);
    let mut trust = SignerTrustPolicy::new();
    enroll(&mut trust, fix.verifying, "prod");
    let mut store = FakeObjectStore::new();
    let ingress = StreamedIngress::default();
    let short = &fix.bytes[..fix.bytes.len() / 2];
    let mut reader = Cursor::new(short.to_vec());
    let err = ingress
        .accept_known_length(
            &mut store,
            &trust,
            fix.cluster,
            fix.capsule,
            "prod",
            0,
            fix.bytes.len() as u64,
            &mut reader,
        )
        .unwrap_err();
    assert!(matches!(err, IngressError::Truncated { .. }));
}

#[test]
fn oversize_content_length_rejected_before_read() {
    let fix = build_sealed(5);
    let mut trust = SignerTrustPolicy::new();
    enroll(&mut trust, fix.verifying, "prod");
    let mut store = FakeObjectStore::new();
    let ingress = StreamedIngress::new(IngressLimits {
        max_capsule_bytes: 100,
        max_chunk_bytes: 64,
    });
    let mut reader = Cursor::new(fix.bytes.clone());
    let err = ingress
        .accept_known_length(
            &mut store,
            &trust,
            fix.cluster,
            fix.capsule,
            "prod",
            0,
            fix.bytes.len() as u64,
            &mut reader,
        )
        .unwrap_err();
    assert!(matches!(err, IngressError::Oversize { .. }));
}

#[test]
fn untrusted_signer_rejected() {
    let fix = build_sealed(6);
    let trust = SignerTrustPolicy::new(); // no enrollment
    let mut store = FakeObjectStore::new();
    let ingress = StreamedIngress::default();
    let mut reader = Cursor::new(fix.bytes.clone());
    let err = ingress
        .accept_known_length(
            &mut store,
            &trust,
            fix.cluster,
            fix.capsule,
            "prod",
            0,
            fix.bytes.len() as u64,
            &mut reader,
        )
        .unwrap_err();
    assert!(matches!(err, IngressError::Trust(_)));
}

#[test]
fn wrong_namespace_denied() {
    let fix = build_sealed(7);
    let mut trust = SignerTrustPolicy::new();
    enroll(&mut trust, fix.verifying, "prod");
    let mut store = FakeObjectStore::new();
    let ingress = StreamedIngress::default();
    let mut reader = Cursor::new(fix.bytes.clone());
    let err = ingress
        .accept_known_length(
            &mut store,
            &trust,
            fix.cluster,
            fix.capsule,
            "other",
            0,
            fix.bytes.len() as u64,
            &mut reader,
        )
        .unwrap_err();
    assert!(matches!(err, IngressError::Trust(_)));
}
