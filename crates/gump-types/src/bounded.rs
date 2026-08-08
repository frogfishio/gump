//! Length-bounded strings and D002 human labels.

use core::fmt;
use std::borrow::Cow;

/// Fallible construction for D002 labels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LabelError {
    Empty,
    TooLong { len: usize, max: usize },
    InvalidByte { index: usize, byte: u8 },
    LeadingHyphen,
    TrailingHyphen,
}

impl fmt::Display for LabelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "label is empty"),
            Self::TooLong { len, max } => write!(f, "label length {len} exceeds max {max}"),
            Self::InvalidByte { index, byte } => {
                write!(f, "invalid label byte 0x{byte:02x} at index {index}")
            }
            Self::LeadingHyphen => write!(f, "label must not start with '-'"),
            Self::TrailingHyphen => write!(f, "label must not end with '-'"),
        }
    }
}

impl std::error::Error for LabelError {}

/// UTF-8 string with an explicit byte-length ceiling (fail-closed at construction).
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct BoundedString<const MAX: usize> {
    inner: String,
}

impl<const MAX: usize> BoundedString<MAX> {
    pub fn try_from_str(s: &str) -> Result<Self, LabelError> {
        let len = s.len();
        if len > MAX {
            return Err(LabelError::TooLong { len, max: MAX });
        }
        Ok(Self {
            inner: s.to_owned(),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.inner
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl<const MAX: usize> fmt::Debug for BoundedString<MAX> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("BoundedString").field(&self.inner).finish()
    }
}

impl<const MAX: usize> fmt::Display for BoundedString<MAX> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.inner)
    }
}

impl<const MAX: usize> AsRef<str> for BoundedString<MAX> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// D002 human label: `[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?`, at most 63 bytes.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Label(BoundedString<63>);

impl Label {
    pub fn parse(input: impl AsRef<str>) -> Result<Self, LabelError> {
        let s = input.as_ref();
        if s.is_empty() {
            return Err(LabelError::Empty);
        }
        if s.len() > 63 {
            return Err(LabelError::TooLong {
                len: s.len(),
                max: 63,
            });
        }
        let bytes = s.as_bytes();
        if bytes[0] == b'-' {
            return Err(LabelError::LeadingHyphen);
        }
        if bytes[bytes.len() - 1] == b'-' {
            return Err(LabelError::TrailingHyphen);
        }
        for (index, &byte) in bytes.iter().enumerate() {
            let ok = matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-');
            if !ok {
                return Err(LabelError::InvalidByte { index, byte });
            }
        }
        // Single-char labels are allowed by the regex when they match [a-z0-9].
        Ok(Self(BoundedString::try_from_str(s)?))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn into_cow(self) -> Cow<'static, str> {
        Cow::Owned(self.0.inner)
    }
}

impl fmt::Debug for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Label").field(&self.as_str()).finish()
    }
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for Label {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_accepts_valid_names() {
        let max = "x".repeat(63);
        for s in ["a", "app", "accounts-service", "a1", max.as_str()] {
            Label::parse(s).unwrap_or_else(|e| panic!("expected ok for {s:?}: {e}"));
        }
    }

    #[test]
    fn label_rejects_invalid_names() {
        assert_eq!(Label::parse(""), Err(LabelError::Empty));
        assert!(matches!(
            Label::parse("-bad"),
            Err(LabelError::LeadingHyphen)
        ));
        assert!(matches!(
            Label::parse("bad-"),
            Err(LabelError::TrailingHyphen)
        ));
        assert!(matches!(
            Label::parse("Bad"),
            Err(LabelError::InvalidByte { .. })
        ));
        assert!(matches!(
            Label::parse("a".repeat(64)),
            Err(LabelError::TooLong { .. })
        ));
    }

    #[test]
    fn bounded_string_enforces_max() {
        assert!(BoundedString::<3>::try_from_str("abcd").is_err());
        assert_eq!(
            BoundedString::<3>::try_from_str("abc").unwrap().as_str(),
            "abc"
        );
    }
}
