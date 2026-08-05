//! Pack/unpack API for `ustar+zstd/1`.

use std::io::{Read, Write};

use super::error::{ArchiveError, ArchiveErrorKind};
use super::extract::ExtractLimits;
use super::path::validate_archive_path;
use super::ustar::{parse_ustar, write_ustar};

/// Archive format identifier carried by `ArchiveMetadataV1`.
pub const ARCHIVE_FORMAT: &str = "ustar+zstd/1";

/// Zstandard compression level required by FORMATS.md §6.
pub const ZSTD_LEVEL: i32 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum EntryKind {
    File,
    Directory,
}

/// One normalized archive entry prior to ustar encoding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveEntry {
    pub path: String,
    pub kind: EntryKind,
    pub executable: bool,
    pub data: Vec<u8>,
}

impl ArchiveEntry {
    pub fn file(
        path: impl Into<String>,
        data: impl Into<Vec<u8>>,
        executable: bool,
    ) -> Result<Self, ArchiveError> {
        let path = validate_archive_path(&path.into(), false)?;
        Ok(Self {
            path,
            kind: EntryKind::File,
            executable,
            data: data.into(),
        })
    }

    pub fn directory(path: impl Into<String>) -> Result<Self, ArchiveError> {
        let path = validate_archive_path(&path.into(), true)?;
        Ok(Self {
            path,
            kind: EntryKind::Directory,
            executable: false,
            data: Vec::new(),
        })
    }
}

/// Write deterministic ustar bytes then compress as one Zstandard frame.
pub fn pack_archive(entries: &[ArchiveEntry]) -> Result<Vec<u8>, ArchiveError> {
    let ustar = write_ustar(entries)?;
    compress_ustar(&ustar)
}

/// Decompress then parse ustar under extract ceilings.
pub fn unpack_archive(
    bytes: &[u8],
    limits: &ExtractLimits,
) -> Result<Vec<ArchiveEntry>, ArchiveError> {
    let ustar = decompress_ustar(bytes, limits.max_uncompressed_bytes)?;
    parse_ustar(&ustar, limits.max_files, limits.max_uncompressed_bytes)
}

/// Compress ustar with FORMATS.md §6 Zstandard parameters.
pub fn compress_ustar(ustar: &[u8]) -> Result<Vec<u8>, ArchiveError> {
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), ZSTD_LEVEL)
        .map_err(|e| ArchiveError::new(ArchiveErrorKind::Compress, e.to_string()))?;
    encoder
        .include_checksum(true)
        .map_err(|e| ArchiveError::new(ArchiveErrorKind::Compress, e.to_string()))?;
    encoder
        .include_contentsize(true)
        .map_err(|e| ArchiveError::new(ArchiveErrorKind::Compress, e.to_string()))?;
    encoder
        .include_dictid(false)
        .map_err(|e| ArchiveError::new(ArchiveErrorKind::Compress, e.to_string()))?;
    encoder
        .set_pledged_src_size(Some(ustar.len() as u64))
        .map_err(|e| ArchiveError::new(ArchiveErrorKind::Compress, e.to_string()))?;
    encoder
        .write_all(ustar)
        .map_err(|e| ArchiveError::new(ArchiveErrorKind::Compress, e.to_string()))?;
    encoder
        .finish()
        .map_err(|e| ArchiveError::new(ArchiveErrorKind::Compress, e.to_string()))
}

/// Decompress a single Zstandard frame with an uncompressed-size ceiling.
pub fn decompress_ustar(compressed: &[u8], max_uncompressed: u64) -> Result<Vec<u8>, ArchiveError> {
    let mut decoder = zstd::stream::Decoder::new(std::io::Cursor::new(compressed))
        .map_err(|e| ArchiveError::new(ArchiveErrorKind::Compress, e.to_string()))?;
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = decoder
            .read(&mut buf)
            .map_err(|e| ArchiveError::new(ArchiveErrorKind::Compress, e.to_string()))?;
        if n == 0 {
            break;
        }
        let next = out.len() as u64 + n as u64;
        if next > max_uncompressed {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Limit,
                "uncompressed size ceiling exceeded",
            ));
        }
        out.extend_from_slice(&buf[..n]);
    }
    Ok(out)
}
