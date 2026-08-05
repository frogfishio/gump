//! CLI error types.

use core::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CliErrorKind {
    Io,
    Manifest,
    Capture,
    Archive,
    Crypto,
    Capsule,
    Driver,
    Policy,
    Args,
}

impl fmt::Display for CliErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Io => "io",
            Self::Manifest => "manifest",
            Self::Capture => "capture",
            Self::Archive => "archive",
            Self::Crypto => "crypto",
            Self::Capsule => "capsule",
            Self::Driver => "driver",
            Self::Policy => "policy",
            Self::Args => "args",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliError {
    kind: CliErrorKind,
    message: String,
}

impl CliError {
    pub fn new(kind: CliErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> CliErrorKind {
        self.kind
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for CliError {}
