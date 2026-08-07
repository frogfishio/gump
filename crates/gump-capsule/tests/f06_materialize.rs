//! F06 / STL-06 materialization: exclusive staging + no-replace publish.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::thread;
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

fn sample_archive() -> Vec<u8> {
    pack_archive(&[
        ArchiveEntry::directory("bin").unwrap(),
        ArchiveEntry::file("bin/hello", b"#!/bin/sh\nexit 0\n", true).unwrap(),
        ArchiveEntry::file("README", b"ok\n", false).unwrap(),
    ])
    .unwrap()
}

#[test]
fn materializes_archive_under_apps_capsule_id() {
    let state = tmp("state");
    let archive = sample_archive();
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
    // STL-06: failed second attempt must not wipe the winner.
    assert_eq!(
        fs::read(mat.root.join("bin/hello")).unwrap(),
        b"#!/bin/sh\nexit 0\n"
    );
    let _ = fs::remove_dir_all(state);
}

#[test]
fn concurrent_same_capsule_one_winner_preserves_publish() {
    let state = tmp("concurrent");
    let archive = Arc::new(sample_archive());
    let capsule = CapsuleId::new();
    let n = 8;
    let barrier = Arc::new(Barrier::new(n));
    let mut handles = Vec::with_capacity(n);
    for _ in 0..n {
        let state = state.clone();
        let archive = Arc::clone(&archive);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            materialize_application_archive(
                &state,
                capsule,
                archive.as_ref(),
                &ExtractLimits::default(),
            )
        }));
    }

    let mut wins = 0usize;
    let mut losses = 0usize;
    for h in handles {
        match h.join().unwrap() {
            Ok(mat) => {
                wins += 1;
                assert_eq!(mat.capsule_id, capsule);
                assert_eq!(
                    fs::read(mat.root.join("bin/hello")).unwrap(),
                    b"#!/bin/sh\nexit 0\n"
                );
            }
            Err(e) => {
                losses += 1;
                assert_eq!(e.kind(), ArchiveErrorKind::Io);
            }
        }
    }
    assert_eq!(wins, 1, "exactly one concurrent publish must win");
    assert_eq!(losses, n - 1);

    let root = state.join("apps").join(capsule.to_hyphenated());
    assert!(root.is_dir());
    assert_eq!(
        fs::read(root.join("bin/hello")).unwrap(),
        b"#!/bin/sh\nexit 0\n"
    );
    // No leftover exclusive staging dirs under apps.
    for ent in fs::read_dir(state.join("apps")).unwrap() {
        let name = ent.unwrap().file_name();
        let s = name.to_string_lossy();
        assert!(
            !s.starts_with(".staging-"),
            "staging leak after concurrent materialize: {s}"
        );
    }
    let _ = fs::remove_dir_all(state);
}

#[test]
fn failed_extract_does_not_create_target_or_leave_staging() {
    let state = tmp("bad-archive");
    let capsule = CapsuleId::new();
    let target = state.join("apps").join(capsule.to_hyphenated());
    let err = materialize_application_archive(
        &state,
        capsule,
        b"not-a-valid-archive",
        &ExtractLimits::default(),
    )
    .unwrap_err();
    assert!(
        matches!(
            err.kind(),
            ArchiveErrorKind::Format | ArchiveErrorKind::Compress | ArchiveErrorKind::Io
        ),
        "unexpected kind {:?}",
        err.kind()
    );
    assert!(
        !target.exists(),
        "failed materialize must not publish target"
    );
    if state.join("apps").is_dir() {
        for ent in fs::read_dir(state.join("apps")).unwrap() {
            let name = ent.unwrap().file_name();
            let s = name.to_string_lossy();
            assert!(
                !s.starts_with(".staging-"),
                "staging must be cleaned on failure: {s}"
            );
        }
    }
    let _ = fs::remove_dir_all(state);
}
