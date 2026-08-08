//! Compile `proto/gump/v1/*.proto` into Rust via prost (DECISIONS D001).

use std::io::Result;
use std::path::PathBuf;

fn main() -> Result<()> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let proto_root = manifest_dir
        .join("../..")
        .join("proto")
        .canonicalize()
        .expect("proto/ directory");

    let formats = proto_root.join("gump/v1/formats.proto");
    let cluster = proto_root.join("gump/v1/cluster.proto");
    let hiccup = proto_root.join("gump/v1/hiccup.proto");

    println!("cargo:rerun-if-changed={}", formats.display());
    println!("cargo:rerun-if-changed={}", cluster.display());
    println!("cargo:rerun-if-changed={}", hiccup.display());

    let protoc = protoc_bin_vendored::protoc_bin_path().map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("vendored protoc unavailable: {e}"),
        )
    })?;

    let mut config = prost_build::Config::new();
    config.protoc_executable(protoc);
    // Keep wire bytes as Vec<u8> for exact golden compares.
    config.compile_protos(&[formats, cluster, hiccup], &[proto_root])?;
    Ok(())
}
