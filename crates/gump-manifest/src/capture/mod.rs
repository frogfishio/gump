//! Workspace capture into a virtual package tree (DELIVERY F02).
//!
//! Authority: docs/v1/FORMATS.md §11, DECISIONS D009.

mod deny;
mod plan;
mod tree;

pub use deny::{is_sensitive_relative_path, SensitiveDeny};
pub use plan::{CaptureError, CaptureErrorKind, CapturePlan};
pub use tree::{
    apply_prepare_outputs, capture_workspace, FileIdentity, VirtualEntry, VirtualTree,
};
