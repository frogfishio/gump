//! STL-03: streaming Capsule verify keeps peak buffers far below Capsule size.
//!
//! Evidence: a Capsule whose application archive is multi-megabyte verifies with
//! a fixed-size chunk; peak_buffer_bytes stays O(chunk + table), not O(capsule).

use gump_capsule::{GumpCapsuleHeader, StreamingCapsuleReader, TABLE_BYTE_LEN, write_gump_capsule};

#[test]
fn large_archive_verify_peak_is_chunk_bounded() {
    let archive = vec![0xABu8; 2 * 1024 * 1024]; // 2 MiB segment
    let header = GumpCapsuleHeader {
        capsule_id: [0x11; 16],
        cluster_id: [0x22; 16],
        release_signer: "0123456789abcdef".into(),
        created_unix_ms: 1,
    };
    let mut out = Vec::new();
    write_gump_capsule(
        &mut out,
        &header,
        [
            b"meta".as_slice(),
            archive.as_slice(),
            b"prot",
            b"keye",
            b"signatur",
        ],
        [4, archive.len() as u64, 4, 4, 8],
    )
    .unwrap();
    assert!(out.len() > 2 * 1024 * 1024);

    let chunk = 8 * 1024;
    let meta = StreamingCapsuleReader::with_chunk_bytes(out.as_slice(), chunk)
        .verify()
        .unwrap();
    assert_eq!(meta.header.capsule_id, header.capsule_id);
    assert!(
        meta.peak_buffer_bytes <= chunk.max(TABLE_BYTE_LEN as usize),
        "peak {} exceeded bound",
        meta.peak_buffer_bytes
    );
    assert!(
        meta.peak_buffer_bytes * 8 < out.len(),
        "peak must stay well below full Capsule (peak={}, capsule={})",
        meta.peak_buffer_bytes,
        out.len()
    );
}
