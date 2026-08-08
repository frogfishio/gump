//! Re-export shared local framing from `gump-cli` (GUMP-N006).

pub use gump_cli::{FrameError, MAX_FRAME_BYTES, read_frame, write_frame};
