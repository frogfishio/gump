//! F03 exit evidence: byte goldens, bomb/escape rejection.
//!
//! Authority: docs/v1/DELIVERY.md F03, docs/v1/FORMATS.md §6.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use gump_capsule::archive::{
    ARCHIVE_FORMAT, ArchiveEntry, ArchiveErrorKind, ExtractLimits, extract_entries, pack_archive,
    unpack_archive, write_ustar,
};

fn sample_entries() -> Vec<ArchiveEntry> {
    vec![
        ArchiveEntry::directory("bin").unwrap(),
        ArchiveEntry::file("bin/hello", b"hello-gump\n", true).unwrap(),
        ArchiveEntry::file("README", b"gump archive\n", false).unwrap(),
    ]
}

#[test]
fn format_id_is_stable() {
    assert_eq!(ARCHIVE_FORMAT, "ustar+zstd/1");
}

#[test]
fn pack_roundtrip_is_deterministic() {
    let entries = sample_entries();
    let a = pack_archive(&entries).unwrap();
    let b = pack_archive(&entries).unwrap();
    assert_eq!(a, b, "compressed archive must be byte-stable");

    let limits = ExtractLimits::default();
    let out = unpack_archive(&a, &limits).unwrap();
    let mut expected = entries;
    expected.sort_by(|x, y| x.path.as_bytes().cmp(y.path.as_bytes()));
    assert_eq!(out, expected);
}

#[test]
fn golden_archive_matches_checked_in_vector() {
    let golden_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../spec/v1/vectors/archive/minimal.ustar.zst");
    let produced = pack_archive(&sample_entries()).unwrap();
    if std::env::var_os("GUMP_WRITE_GOLDEN").is_some() {
        fs::create_dir_all(golden_path.parent().unwrap()).unwrap();
        fs::write(&golden_path, &produced).unwrap();
    }
    let golden = fs::read(&golden_path).unwrap_or_else(|e| {
        panic!(
            "missing golden {}: {e}; re-run with GUMP_WRITE_GOLDEN=1",
            golden_path.display()
        )
    });
    assert_eq!(
        produced,
        golden,
        "archive bytes drifted from {} (set GUMP_WRITE_GOLDEN=1 to refresh)",
        golden_path.display()
    );
}

#[test]
fn ustar_entries_are_lexically_sorted() {
    let mut entries = sample_entries();
    entries.reverse();
    let ustar = write_ustar(&entries).unwrap();
    let parsed = gump_capsule::archive::parse_ustar(&ustar, 100, 1 << 20).unwrap();
    let paths: Vec<_> = parsed.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, vec!["README", "bin/", "bin/hello"]);
}

#[test]
fn rejects_path_escape_on_pack() {
    let err = ArchiveEntry::file("../etc/passwd", b"x", false).unwrap_err();
    assert_eq!(err.kind(), ArchiveErrorKind::Escape);
}

#[test]
fn rejects_symlink_typeflag_in_ustar() {
    // Craft a minimal bad header: copy a good archive and flip typeflag.
    let ustar = write_ustar(&sample_entries()).unwrap();
    let mut evil = ustar.clone();
    evil[156] = b'2'; // symlink
    // Fix checksum for the mutated header so we reach typeflag validation.
    evil[148..156].fill(b' ');
    let sum: u32 = evil[..512].iter().map(|&b| b as u32).sum();
    let s = format!("{:06o}", sum);
    evil[148..154].copy_from_slice(s.as_bytes());
    evil[154] = 0;
    evil[155] = b' ';
    let err = gump_capsule::archive::parse_ustar(&evil, 100, 1 << 20).unwrap_err();
    assert_eq!(err.kind(), ArchiveErrorKind::Format);
}

#[test]
fn bomb_file_count_ceiling() {
    let entries = sample_entries();
    let packed = pack_archive(&entries).unwrap();
    let limits = ExtractLimits {
        max_files: 1,
        max_uncompressed_bytes: 1 << 20,
        max_path_bytes: 4096,
    };
    let err = unpack_archive(&packed, &limits).unwrap_err();
    assert_eq!(err.kind(), ArchiveErrorKind::Limit);
}

#[test]
fn bomb_uncompressed_ceiling() {
    let entries = sample_entries();
    let packed = pack_archive(&entries).unwrap();
    let limits = ExtractLimits {
        max_files: 100,
        max_uncompressed_bytes: 8,
        max_path_bytes: 4096,
    };
    let err = unpack_archive(&packed, &limits).unwrap_err();
    assert_eq!(err.kind(), ArchiveErrorKind::Limit);
}

#[test]
fn extract_rejects_escape_via_dotdot_segments() {
    // Direct extract API also validates paths.
    let staging = tmp_dir("escape");
    let bad = ArchiveEntry {
        path: "../outside".into(),
        kind: gump_capsule::archive::EntryKind::File,
        executable: false,
        data: b"x".to_vec(),
    };
    let err = extract_entries(&staging, &[bad], &ExtractLimits::default()).unwrap_err();
    assert!(matches!(
        err.kind(),
        ArchiveErrorKind::Escape | ArchiveErrorKind::Path
    ));
    let _ = fs::remove_dir_all(staging);
}

#[test]
fn extract_writes_files_under_staging() {
    let staging = tmp_dir("ok");
    let entries = sample_entries();
    extract_entries(&staging, &entries, &ExtractLimits::default()).unwrap();
    assert_eq!(
        fs::read(staging.join("bin/hello")).unwrap(),
        b"hello-gump\n"
    );
    assert_eq!(fs::read(staging.join("README")).unwrap(), b"gump archive\n");
    let _ = fs::remove_dir_all(staging);
}

fn tmp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("gump-f03-{name}-{nanos}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}
