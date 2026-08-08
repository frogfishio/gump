//! Deterministic application archive: POSIX ustar + Zstandard (DELIVERY F03).
//!
//! Authority: docs/v1/FORMATS.md §6, DECISIONS D003/D009.

mod error;
mod extract;
mod materialize;
mod pack;
mod path;
mod ustar;

pub use error::{ArchiveError, ArchiveErrorKind};
pub use extract::{
    ExtractLimits, extract_entries, extract_ustar_from_reader, extract_ustar_zstd_from_reader,
};
pub use materialize::{MaterializedRelease, materialize_application_archive};
pub use pack::{
    ARCHIVE_FORMAT, ArchiveEntry, EntryKind, compress_ustar, decompress_ustar, pack_archive,
    pack_archive_to, unpack_archive,
};
pub use path::validate_archive_path;
pub use ustar::{parse_ustar, ustar_encoded_len, write_ustar, write_ustar_to};
