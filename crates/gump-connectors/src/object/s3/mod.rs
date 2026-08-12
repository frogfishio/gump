//! S3-compatible object connector (D02 / DECISIONS D008 / STL-07).
//!
//! Wraps `aws-sdk-s3` (SigV4, TLS, credential chain, retries, multipart) behind
//! [`crate::object::ObjectStore`]. User metadata `gump-blake3` carries digest
//! evidence; publication uses a capability-selected conditional copy or put.

mod client;

pub use client::{META_BLAKE3, S3Config, S3ObjectStore, S3PublishStrategy, S3ReadStats};
