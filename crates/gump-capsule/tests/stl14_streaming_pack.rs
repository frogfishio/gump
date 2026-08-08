//! STL-14 evidence: streaming pack + materialize without a complete archive slice.
//!
//! Authority: docs/v1/CONFORMANCE.md archive extraction, FORMATS.md §6.

use std::fs::{self, File};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use gump_capsule::archive::{
    ArchiveEntry, ExtractLimits, materialize_application_archive, pack_archive_to, write_ustar,
    write_ustar_to,
};
use gump_types::CapsuleId;

fn tmp(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("gump-stl14-{name}-{nanos}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn multi_mib_pack_to_file_materialize_from_reader() {
    let size = 4 * 1024 * 1024;
    let payload = vec![0xA5u8; size];
    let entries = vec![
        ArchiveEntry::directory("bin").unwrap(),
        ArchiveEntry::file("bin/big.bin", payload, false).unwrap(),
        ArchiveEntry::file("README", b"stl14\n", false).unwrap(),
    ];

    let root = tmp("roundtrip");
    let packed_path = root.join("archive.ustar.zst");
    {
        let file = File::create(&packed_path).unwrap();
        // Streams ustar→zstd into the file sink (no full compressed Vec return).
        pack_archive_to(&entries, file).unwrap();
    }

    // Free entry payloads before materialize — API takes a Read, not `&[u8]`.
    drop(entries);

    let state = root.join("state");
    let reader = File::open(&packed_path).unwrap();
    let mat = materialize_application_archive(
        &state,
        CapsuleId::new(),
        reader,
        &ExtractLimits::default(),
    )
    .unwrap();

    assert_eq!(
        fs::metadata(mat.root.join("bin/big.bin")).unwrap().len(),
        size as u64
    );
    assert_eq!(fs::read(mat.root.join("README")).unwrap(), b"stl14\n");
    assert_eq!(mat.file_count, 3);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn write_ustar_to_matches_buffered_write_ustar() {
    let entries = [
        ArchiveEntry::directory("a").unwrap(),
        ArchiveEntry::file("a/f", b"hello", false).unwrap(),
    ];
    let buffered = write_ustar(&entries).unwrap();
    let mut streamed = Vec::new();
    write_ustar_to(&entries, &mut streamed).unwrap();
    assert_eq!(buffered, streamed);
}
