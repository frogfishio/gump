//! Canonical topic validation (RUNTIME.md §14).

use core::fmt;

/// Maximum topic length in bytes.
pub const MAX_TOPIC_LEN: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TopicError {
    Empty,
    TooLong { len: usize },
    InvalidByte { index: usize, byte: u8 },
    EmptySegment,
    ReservedImpersonation,
}

impl fmt::Display for TopicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "topic is empty"),
            Self::TooLong { len } => write!(f, "topic length {len} exceeds {MAX_TOPIC_LEN}"),
            Self::InvalidByte { index, byte } => {
                write!(f, "invalid topic byte 0x{byte:02x} at {index}")
            }
            Self::EmptySegment => write!(f, "topic has empty slash segment"),
            Self::ReservedImpersonation => {
                write!(f, "application cannot emit reserved gump/ topics via forgery path")
            }
        }
    }
}

impl std::error::Error for TopicError {}

/// Canonical topics: lowercase ASCII, 1–128 bytes, slash-separated.
pub fn validate_topic(topic: &str) -> Result<(), TopicError> {
    if topic.is_empty() {
        return Err(TopicError::Empty);
    }
    if topic.len() > MAX_TOPIC_LEN {
        return Err(TopicError::TooLong { len: topic.len() });
    }
    if topic.starts_with('/') || topic.ends_with('/') || topic.contains("//") {
        return Err(TopicError::EmptySegment);
    }
    for (index, &byte) in topic.as_bytes().iter().enumerate() {
        let ok = matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'/' | b'_' | b'-' | b':' | b'*');
        if !ok {
            return Err(TopicError::InvalidByte { index, byte });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_runtime_topics() {
        for t in ["app/stdout", "app/stderr", "gump/lifecycle", "app:event"] {
            validate_topic(t).unwrap();
        }
    }

    #[test]
    fn rejects_upper_and_empty() {
        assert!(validate_topic("App/Stdout").is_err());
        assert!(validate_topic("").is_err());
        assert!(validate_topic("a//b").is_err());
    }
}
