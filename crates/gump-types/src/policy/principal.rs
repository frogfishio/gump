//! Provider-qualified principal IDs.

use crate::bounded::{BoundedString, LabelError};

/// Stable provider-qualified principal (`provider:subject`).
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PrincipalId(BoundedString<256>);

impl PrincipalId {
    pub fn new(id: impl AsRef<str>) -> Result<Self, LabelError> {
        let id = id.as_ref();
        if id.is_empty() {
            return Err(LabelError::Empty);
        }
        if !id.contains(':') {
            return Err(LabelError::InvalidByte {
                index: 0,
                byte: b'?',
            });
        }
        Ok(Self(BoundedString::try_from_str(id)?))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl std::fmt::Display for PrincipalId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
