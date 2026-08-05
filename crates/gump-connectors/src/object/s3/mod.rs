//! S3-compatible object connector (D02 / DECISIONS D008).
//!
//! Speaks path-style HTTP PUT/GET/HEAD/DELETE with `If-None-Match: *` for
//! immutable final publication and `x-amz-meta-gump-blake3` for digest evidence.

mod client;
mod http;

pub use client::{S3Config, S3ObjectStore};
pub use http::{S3HttpError, S3ObjectMeta};
