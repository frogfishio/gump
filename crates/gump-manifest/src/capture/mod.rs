//! Workspace capture into a virtual package tree (DELIVERY F02).
//!
//! Authority: docs/v1/FORMATS.md §11, DECISIONS D009.

mod deny;
mod plan;
mod tree;

pub use deny::{SensitiveDeny, is_sensitive_relative_path};
pub use plan::{CaptureError, CaptureErrorKind, CapturePlan};
pub use tree::{
    FileIdentity, VirtualEntry, VirtualTree, apply_prepare_outputs, capture_workspace,
    verify_captured_bytes,
};
