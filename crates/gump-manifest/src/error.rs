//! Manifest parse / validation errors.

use core::fmt;

/// Stable classification for manifest failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ManifestErrorKind {
    Toml,
    Schema,
    UnknownKey,
    MissingField,
    InvalidValue,
    Semantic,
}

impl fmt::Display for ManifestErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Toml => "toml",
            Self::Schema => "schema",
            Self::UnknownKey => "unknown_key",
            Self::MissingField => "missing_field",
            Self::InvalidValue => "invalid_value",
            Self::Semantic => "semantic",
        })
    }
}

/// Fail-closed manifest error (no secret material; path + kind only).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestError {
    kind: ManifestErrorKind,
    path: String,
    message: String,
}

impl ManifestError {
    pub fn new(
        kind: ManifestErrorKind,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            path: path.into(),
            message: message.into(),
        }
    }

    pub fn kind(&self) -> ManifestErrorKind {
        self.kind
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}: {}", self.kind, self.path, self.message)
    }
}

impl std::error::Error for ManifestError {}
