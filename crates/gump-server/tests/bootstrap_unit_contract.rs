//! Canonical inactive host contract consumed by Captain.

use sha2::{Digest as _, Sha256};
use std::fmt::Write as _;

#[test]
fn bootstrap_unit_is_inactive_secret_free_and_byte_stable() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap();
    let bytes = std::fs::read(root.join("packaging/systemd/gump-bootstrap.service")).unwrap();
    let mut digest = String::with_capacity(64);
    for byte in Sha256::digest(&bytes) {
        write!(&mut digest, "{byte:02x}").unwrap();
    }
    assert_eq!(
        digest,
        "545f853e5602d555429f87402a5a31a9988ccb982a28b67ea79fc1205799c296"
    );
    let unit = std::str::from_utf8(&bytes).unwrap();
    for required in [
        "User=gump",
        "Group=gump",
        "UMask=0077",
        "RuntimeDirectory=gump",
        "RuntimeDirectoryMode=0700",
        "RuntimeDirectoryPreserve=no",
        "Restart=no",
        "/usr/bin/gump server --bootstrap",
    ] {
        assert!(unit.contains(required), "unit is missing {required}");
    }
    for forbidden in [
        "WantedBy=",
        "activationCode",
        "RECOVERY_SECRET",
        "ACCESS_KEY",
        "SECRET_KEY",
    ] {
        assert!(
            !unit.contains(forbidden),
            "unit contains forbidden {forbidden}"
        );
    }
}
