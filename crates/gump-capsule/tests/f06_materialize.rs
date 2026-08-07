//! F06 materialization: archive → `<state-root>/apps/<capsule-id>/`.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use gump_capsule::archive::{
    ArchiveEntry, ArchiveErrorKind, ExtractLimits, materialize_application_archive, pack_archive,
};
use gump_types::CapsuleId;

fn tmp(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("gump-f06-mat-{name}-{nanos}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn materializes_archive_under_apps_capsule_id() {
    let state = tmp("state");
    let archive = pack_archive(&[
        ArchiveEntry::directory("bin").unwrap(),
        ArchiveEntry::file("bin/hello", b"#!/bin/sh\nexit 0\n", true).unwrap(),
        ArchiveEntry::file("README", b"ok\n", false).unwrap(),
    ])
    .unwrap();
    let capsule = CapsuleId::new();
    let mat = materialize_application_archive(&state, capsule, &archive, &ExtractLimits::default())
        .unwrap();
    assert_eq!(mat.capsule_id, capsule);
    assert_eq!(mat.root, state.join("apps").join(capsule.to_hyphenated()));
    assert_eq!(
        fs::read(mat.root.join("bin/hello")).unwrap(),
        b"#!/bin/sh\nexit 0\n"
    );
    // Second materialize of same id fails (cache exists).
    let err = materialize_application_archive(&state, capsule, &archive, &ExtractLimits::default())
        .unwrap_err();
    assert_eq!(err.kind(), ArchiveErrorKind::Io);
    let _ = fs::remove_dir_all(state);
}
