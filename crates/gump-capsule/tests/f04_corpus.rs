//! F04 exit evidence: capsule-lib cross-goldens + malformed GUMPDEP1 table corpus.

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use capsule_lib::{Capsule, Encoding, ParseOptions};
use gump_capsule::{
    GumpCapsuleHeader, SEGMENT_DESC_LEN, SegmentTable, SegmentType, StreamingCapsuleReader,
    StreamingCapsuleWriter, TABLE_PREFIX_LEN, read_gump_capsule, write_gump_capsule,
};

fn vectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec/v1/vectors/capsule")
}

fn sample_header() -> GumpCapsuleHeader {
    GumpCapsuleHeader {
        capsule_id: [0xAA; 16],
        cluster_id: [0xBB; 16],
        release_signer: "deadbeefcafebabe".into(),
        created_unix_ms: 1_714_000_000_000,
    }
}

#[test]
fn capsule_lib_cbor_golden_parses_with_reference() {
    let bytes = fs::read(vectors_dir().join("capsule-lib-cbor.capsule")).unwrap();
    let decoded = Capsule::parse_with_options(
        &bytes,
        ParseOptions {
            verify_crc: true,
            validate_encoding: true,
        },
    )
    .expect("capsule-lib golden must parse");
    assert_eq!(decoded.prelude.encoding, Encoding::Cbor);
    assert_eq!(decoded.prelude.version.0, 1);
    // Round-trip through capsule-lib serializer.
    let reserialized = Capsule {
        prelude: decoded.prelude.clone(),
        header_encoded: decoded.header_encoded.clone(),
        payload_encoded: decoded.payload_encoded.clone(),
    }
    .to_bytes()
    .unwrap();
    assert_eq!(bytes, reserialized);
}

#[test]
fn gump_capsule_roundtrips_through_capsule_lib_and_streaming_reader() {
    let mut bytes = Vec::new();
    let written = write_gump_capsule(
        &mut bytes,
        &sample_header(),
        [b"pub", b"app", b"cfg", b"key", b"sig"],
        [3, 3, 3, 3, 3],
    )
    .unwrap();

    // Reference parser agrees on framing.
    let decoded = Capsule::parse(&bytes).unwrap();
    assert_eq!(decoded.prelude.encoding, Encoding::Cbor);
    assert_eq!(
        decoded.header_decoded,
        written.header.encode_cbor().unwrap()
    );

    // Dialect reader verifies segment table.
    let view = read_gump_capsule(&bytes).unwrap();
    assert_eq!(view.segment(SegmentType::PublicMetadata), b"pub");
    assert_eq!(view.segment(SegmentType::ReleaseSignature), b"sig");

    // Streaming reader path (Read → view).
    let streamed = StreamingCapsuleReader::new(Cursor::new(bytes.clone()))
        .read_all()
        .unwrap();
    assert_eq!(streamed.inner, view.inner);

    // Streaming writer path.
    let mut w = StreamingCapsuleWriter::new(sample_header());
    w.set_segment(SegmentType::PublicMetadata, b"pub".to_vec(), 3);
    w.set_segment(SegmentType::ApplicationArchive, b"app".to_vec(), 3);
    w.set_segment(SegmentType::ProtectedConfig, b"cfg".to_vec(), 3);
    w.set_segment(SegmentType::KeyEnvelope, b"key".to_vec(), 3);
    w.set_segment(SegmentType::ReleaseSignature, b"sig".to_vec(), 3);
    let mut out2 = Vec::new();
    w.finish(&mut out2).unwrap();
    assert_eq!(out2, bytes);
}

fn valid_inner() -> Vec<u8> {
    let parts = [
        (SegmentType::PublicMetadata, &b"a"[..], 1u64),
        (SegmentType::ApplicationArchive, &b"b"[..], 1),
        (SegmentType::ProtectedConfig, &b"c"[..], 1),
        (SegmentType::KeyEnvelope, &b"d"[..], 1),
        (SegmentType::ReleaseSignature, &b"e"[..], 1),
    ];
    let table = SegmentTable::from_stored_parts(parts).unwrap();
    let mut inner = table.encode();
    for (_, bytes, _) in parts {
        inner.extend_from_slice(bytes);
    }
    SegmentTable::parse_and_verify(&inner).unwrap();
    inner
}

#[test]
fn malformed_table_corpus_is_rejected() {
    let good = valid_inner();

    // Bad magic.
    let mut bad_magic = good.clone();
    bad_magic[0] = b'X';
    assert!(SegmentTable::parse_and_verify(&bad_magic).is_err());

    // Bad version.
    let mut bad_ver = good.clone();
    bad_ver[9] = 2;
    assert!(SegmentTable::parse_and_verify(&bad_ver).is_err());

    // Bad count.
    let mut bad_count = good.clone();
    bad_count[11] = 4;
    assert!(SegmentTable::parse_and_verify(&bad_count).is_err());

    // Truncated table.
    assert!(SegmentTable::parse_and_verify(&good[..TABLE_PREFIX_LEN + SEGMENT_DESC_LEN]).is_err());

    // Digest mismatch.
    let mut bad_digest = good.clone();
    let seg0_off =
        usize::try_from(SegmentTable::parse_and_verify(&good).unwrap().descriptors[0].offset)
            .unwrap();
    bad_digest[seg0_off] ^= 0xff;
    assert!(SegmentTable::parse_and_verify(&bad_digest).is_err());

    // Gap / non-contiguous: bump offset of segment 2 without moving bytes.
    let mut gapped = good.clone();
    let desc2 = TABLE_PREFIX_LEN + 2 * SEGMENT_DESC_LEN;
    let offset_bytes = &mut gapped[desc2 + 8..desc2 + 16];
    let mut off = u64::from_be_bytes(offset_bytes.try_into().unwrap());
    off += 1;
    offset_bytes.copy_from_slice(&off.to_be_bytes());
    assert!(SegmentTable::parse_and_verify(&gapped).is_err());

    // Unsorted / wrong type in slot 0.
    let mut wrong_type = good.clone();
    wrong_type[TABLE_PREFIX_LEN] = 0;
    wrong_type[TABLE_PREFIX_LEN + 1] = 2; // type 2 in first slot
    assert!(SegmentTable::parse_and_verify(&wrong_type).is_err());

    // Trailing bytes.
    let mut trailing = good.clone();
    trailing.push(0);
    assert!(SegmentTable::parse_and_verify(&trailing).is_err());

    // Non-zero flags.
    let mut flags = good.clone();
    flags[TABLE_PREFIX_LEN + 3] = 1;
    assert!(SegmentTable::parse_and_verify(&flags).is_err());
}
