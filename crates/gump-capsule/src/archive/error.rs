//! Archive error types.

use core::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ArchiveErrorKind {
    Path,
    Escape,
    Format,
    Limit,
    Io,
    Compress,
}

impl fmt::Display for ArchiveErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Path => "path",
            Self::Escape => "escape",
            Self::Format => "format",
            Self::Limit => "limit",
            Self::Io => "io",
            Self::Compress => "compress",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveError {
    kind: ArchiveErrorKind,
    message: String,
}

impl ArchiveError {
    pub fn new(kind: ArchiveErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> ArchiveErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for ArchiveError {}

impl From<std::io::Error> for ArchiveError {
    fn from(value: std::io::Error) -> Self {
        Self::new(ArchiveErrorKind::Io, value.to_string())
    }
}
