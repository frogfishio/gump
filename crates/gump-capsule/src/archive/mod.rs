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
pub use extract::{ExtractLimits, extract_entries};
pub use materialize::{MaterializedRelease, materialize_application_archive};
pub use pack::{
    ARCHIVE_FORMAT, ArchiveEntry, EntryKind, compress_ustar, decompress_ustar, pack_archive,
    unpack_archive,
};
pub use path::validate_archive_path;
pub use ustar::{parse_ustar, write_ustar};
