//! Crypto error types.

use core::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CryptoErrorKind {
    Aead,
    Hpke,
    Signature,
    Encoding,
    Length,
}

impl fmt::Display for CryptoErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Aead => "aead",
            Self::Hpke => "hpke",
            Self::Signature => "signature",
            Self::Encoding => "encoding",
            Self::Length => "length",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CryptoError {
    kind: CryptoErrorKind,
    message: String,
}

impl CryptoError {
    pub fn new(kind: CryptoErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> CryptoErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for CryptoError {}
