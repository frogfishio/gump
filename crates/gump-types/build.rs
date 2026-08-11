// SPDX-FileCopyrightText: 2026 Alexander R. Croft
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let root =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory")).join("../..");
    let version_path = root.join("VERSION");
    let build_path = root.join("BUILD");

    println!("cargo:rerun-if-changed={}", version_path.display());
    println!("cargo:rerun-if-changed={}", build_path.display());
    println!("cargo:rerun-if-env-changed=GUMP_BUILD");

    let version = read_trimmed(&version_path);
    validate_version(&version);
    let build = env::var("GUMP_BUILD").unwrap_or_else(|_| read_trimmed(&build_path));
    validate_build(&build);

    println!("cargo:rustc-env=GUMP_VERSION={version}");
    println!("cargo:rustc-env=GUMP_BUILD={build}");
}

fn read_trimmed(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
        .trim()
        .to_owned()
}

fn validate_version(version: &str) {
    let parts = version.split('.').collect::<Vec<_>>();
    assert!(
        parts.len() == 3
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())),
        "VERSION must contain a numeric SemVer core such as 1.2.3"
    );
}

fn validate_build(build: &str) {
    assert!(
        !build.is_empty() && build.bytes().all(|byte| byte.is_ascii_digit()),
        "BUILD (or GUMP_BUILD) must contain an unsigned integer"
    );
}
