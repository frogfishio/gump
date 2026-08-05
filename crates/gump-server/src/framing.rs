//! Length-prefixed JSON frames over a local stream (1 MiB control-frame budget).

use std::io::{Read, Write};

/// Matches DECISIONS D007 initial control-frame maximum.
pub const MAX_FRAME_BYTES: u32 = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrameError {
    Io(String),
    TooLarge { len: u32, max: u32 },
    Truncated,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::TooLarge { len, max } => write!(f, "frame {len} exceeds max {max}"),
            Self::Truncated => write!(f, "truncated frame"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<std::io::Error> for FrameError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

pub fn write_frame(w: &mut impl Write, payload: &[u8]) -> Result<(), FrameError> {
    let len = u32::try_from(payload.len()).map_err(|_| FrameError::TooLarge {
        len: u32::MAX,
        max: MAX_FRAME_BYTES,
    })?;
    if len > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge {
            len,
            max: MAX_FRAME_BYTES,
        });
    }
    w.write_all(&len.to_be_bytes())?;
    w.write_all(payload)?;
    w.flush()?;
    Ok(())
}

pub fn read_frame(r: &mut impl Read) -> Result<Vec<u8>, FrameError> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            FrameError::Truncated
        } else {
            FrameError::from(e)
        }
    })?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge {
            len,
            max: MAX_FRAME_BYTES,
        });
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            FrameError::Truncated
        } else {
            FrameError::from(e)
        }
    })?;
    Ok(buf)
}
