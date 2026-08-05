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
pub use extract::{extract_entries, ExtractLimits};
pub use materialize::{materialize_application_archive, MaterializedRelease};
pub use pack::{
    compress_ustar, decompress_ustar, pack_archive, unpack_archive, ArchiveEntry, EntryKind,
    ARCHIVE_FORMAT,
};
pub use path::validate_archive_path;
pub use ustar::{parse_ustar, write_ustar};
