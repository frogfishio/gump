//! Control-stream framing: unsigned protobuf varint length + payload.
//!
//! PROTOCOL.md §2: reject lengths over the per-message ceiling **before** allocating.

use core::fmt;

/// Default maximum for ordinary control messages (1 MiB).
pub const MAX_CONTROL_FRAME: usize = 1 << 20;
/// Maximum for error frames (16 KiB).
pub const MAX_ERROR_FRAME: usize = 16 << 10;
/// Maximum for Hello frames (64 KiB).
pub const MAX_HELLO_FRAME: usize = 64 << 10;

/// Which ceiling to apply when reading a framed message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameKind {
    Control,
    Error,
    Hello,
}

impl FrameKind {
    pub const fn max_len(self) -> usize {
        match self {
            Self::Control => MAX_CONTROL_FRAME,
            Self::Error => MAX_ERROR_FRAME,
            Self::Hello => MAX_HELLO_FRAME,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameError {
    Truncated,
    VarintOverflow,
    LengthExceedsCeiling { length: usize, ceiling: usize },
    EmptyFrame,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => write!(f, "truncated frame"),
            Self::VarintOverflow => write!(f, "varint overflow"),
            Self::LengthExceedsCeiling { length, ceiling } => {
                write!(f, "frame length {length} exceeds ceiling {ceiling}")
            }
            Self::EmptyFrame => write!(f, "empty frame payload"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Decode an unsigned protobuf-style varint.
///
/// Returns `(value, bytes_consumed)`.
pub fn decode_varint(buf: &[u8]) -> Result<(u64, usize), FrameError> {
    let mut value: u64 = 0;
    let mut shift = 0u32;
    for (i, byte) in buf.iter().copied().enumerate() {
        if i >= 10 {
            return Err(FrameError::VarintOverflow);
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, i + 1));
        }
        shift += 7;
        if shift >= 64 {
            return Err(FrameError::VarintOverflow);
        }
    }
    Err(FrameError::Truncated)
}

/// Encode an unsigned protobuf-style varint into `out`.
pub fn encode_varint(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
            out.push(byte);
        } else {
            out.push(byte);
            break;
        }
    }
}

/// Peek length prefix and enforce `kind` ceiling without copying the payload.
///
/// Returns `(payload_len, prefix_len)`.
pub fn decode_frame_prefix(buf: &[u8], kind: FrameKind) -> Result<(usize, usize), FrameError> {
    let (len_u64, prefix_len) = decode_varint(buf)?;
    let length = usize::try_from(len_u64).map_err(|_| FrameError::VarintOverflow)?;
    if length == 0 {
        return Err(FrameError::EmptyFrame);
    }
    let ceiling = kind.max_len();
    if length > ceiling {
        return Err(FrameError::LengthExceedsCeiling { length, ceiling });
    }
    let total = prefix_len.checked_add(length).ok_or(FrameError::VarintOverflow)?;
    if buf.len() < total {
        return Err(FrameError::Truncated);
    }
    Ok((length, prefix_len))
}

/// Split a complete frame into `(payload, bytes_consumed)` after ceiling checks.
pub fn split_frame(buf: &[u8], kind: FrameKind) -> Result<(&[u8], usize), FrameError> {
    let (length, prefix_len) = decode_frame_prefix(buf, kind)?;
    let end = prefix_len + length;
    Ok((&buf[prefix_len..end], end))
}

/// Length-prefix `payload` for the wire.
pub fn encode_frame(payload: &[u8], kind: FrameKind) -> Result<Vec<u8>, FrameError> {
    if payload.is_empty() {
        return Err(FrameError::EmptyFrame);
    }
    if payload.len() > kind.max_len() {
        return Err(FrameError::LengthExceedsCeiling {
            length: payload.len(),
            ceiling: kind.max_len(),
        });
    }
    let mut out = Vec::with_capacity(10 + payload.len());
    encode_varint(payload.len() as u64, &mut out);
    out.extend_from_slice(payload);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_round_trip() {
        for value in [0u64, 1, 127, 128, 300, 1 << 20, u64::from(u32::MAX)] {
            let mut buf = Vec::new();
            encode_varint(value, &mut buf);
            let (decoded, n) = decode_varint(&buf).unwrap();
            assert_eq!(decoded, value);
            assert_eq!(n, buf.len());
        }
    }

    #[test]
    fn rejects_over_ceiling_before_payload_needed() {
        // Claim 16KiB+1 for an Error frame using only the length prefix bytes.
        let mut buf = Vec::new();
        encode_varint((MAX_ERROR_FRAME + 1) as u64, &mut buf);
        // No payload bytes attached — ceiling check must still fire.
        let err = decode_frame_prefix(&buf, FrameKind::Error).unwrap_err();
        assert_eq!(
            err,
            FrameError::LengthExceedsCeiling {
                length: MAX_ERROR_FRAME + 1,
                ceiling: MAX_ERROR_FRAME,
            }
        );
    }

    #[test]
    fn accepts_hello_at_limit() {
        let payload = vec![0u8; MAX_HELLO_FRAME];
        let frame = encode_frame(&payload, FrameKind::Hello).unwrap();
        let (got, consumed) = split_frame(&frame, FrameKind::Hello).unwrap();
        assert_eq!(got.len(), MAX_HELLO_FRAME);
        assert_eq!(consumed, frame.len());
    }

    #[test]
    fn rejects_empty_payload() {
        assert_eq!(
            encode_frame(&[], FrameKind::Control),
            Err(FrameError::EmptyFrame)
        );
    }
}
