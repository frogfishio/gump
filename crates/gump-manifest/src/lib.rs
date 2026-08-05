//! Parse, normalize, and validate `gump/1` manifests (DELIVERY F01).
//!
//! Authority: `docs/v1/FORMATS.md` §1/§10, `spec/v1/gump.schema.json`.
//! Unknown keys are errors. Durations and byte sizes are normalized on parse.

#![forbid(unsafe_code)]

mod error;
mod model;
mod normalize;
mod parse;
mod scalar;

pub use error::{ManifestError, ManifestErrorKind};
pub use model::*;
pub use parse::{parse_manifest_str, parse_manifest_value};
pub use scalar::{parse_byte_size, parse_duration_millis};
