//! Transport frame/chunk ceilings (DECISIONS D007 / PROTOCOL.md §2).

use core::fmt;

use gump_protocol::{MAX_CONTROL_FRAME, MAX_ERROR_FRAME, MAX_HELLO_FRAME};

/// Bound set applied before allocating receive buffers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransportLimits {
    pub max_control_frame: usize,
    pub max_error_frame: usize,
    pub max_hello_frame: usize,
    pub max_bulk_chunk: usize,
}

impl Default for TransportLimits {
    fn default() -> Self {
        Self {
            max_control_frame: MAX_CONTROL_FRAME,
            max_error_frame: MAX_ERROR_FRAME,
            max_hello_frame: MAX_HELLO_FRAME,
            // D007: bulk data uses 1 MiB chunks.
            max_bulk_chunk: 1 << 20,
        }
    }
}

impl TransportLimits {
    pub fn check_control(&self, len: usize) -> Result<(), TransportLimitError> {
        Self::check(len, self.max_control_frame, "control")
    }

    pub fn check_hello(&self, len: usize) -> Result<(), TransportLimitError> {
        Self::check(len, self.max_hello_frame, "hello")
    }

    pub fn check_error(&self, len: usize) -> Result<(), TransportLimitError> {
        Self::check(len, self.max_error_frame, "error")
    }

    pub fn check_bulk_chunk(&self, len: usize) -> Result<(), TransportLimitError> {
        Self::check(len, self.max_bulk_chunk, "bulk_chunk")
    }

    fn check(len: usize, ceiling: usize, kind: &'static str) -> Result<(), TransportLimitError> {
        if len == 0 {
            return Err(TransportLimitError::Empty { kind });
        }
        if len > ceiling {
            return Err(TransportLimitError::ExceedsCeiling {
                kind,
                length: len,
                ceiling,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportLimitError {
    Empty { kind: &'static str },
    ExceedsCeiling {
        kind: &'static str,
        length: usize,
        ceiling: usize,
    },
}

impl fmt::Display for TransportLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty { kind } => write!(f, "empty {kind} frame"),
            Self::ExceedsCeiling {
                kind,
                length,
                ceiling,
            } => write!(f, "{kind} length {length} exceeds ceiling {ceiling}"),
        }
    }
}

impl std::error::Error for TransportLimitError {}
