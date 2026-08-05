//! Capsule dialect errors.

use core::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CapsuleDialectErrorKind {
    Framing,
    Header,
    Table,
    Segment,
    Io,
    Limit,
}

impl fmt::Display for CapsuleDialectErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Framing => "framing",
            Self::Header => "header",
            Self::Table => "table",
            Self::Segment => "segment",
            Self::Io => "io",
            Self::Limit => "limit",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapsuleDialectError {
    kind: CapsuleDialectErrorKind,
    message: String,
}

impl CapsuleDialectError {
    pub fn new(kind: CapsuleDialectErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> CapsuleDialectErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for CapsuleDialectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for CapsuleDialectError {}

impl From<std::io::Error> for CapsuleDialectError {
    fn from(value: std::io::Error) -> Self {
        Self::new(CapsuleDialectErrorKind::Io, value.to_string())
    }
}

impl From<capsule_lib::CapsuleError> for CapsuleDialectError {
    fn from(value: capsule_lib::CapsuleError) -> Self {
        Self::new(CapsuleDialectErrorKind::Framing, value.to_string())
    }
}
