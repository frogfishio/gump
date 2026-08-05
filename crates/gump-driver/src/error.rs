//! Driver ABI errors.

use core::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum DriverErrorKind {
    Probe,
    Prepare,
    Admit,
    Start,
    Observe,
    Signal,
    Terminate,
    Cleanup,
    Policy,
    Io,
    NotFound,
    State,
}

impl fmt::Display for DriverErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Probe => "probe",
            Self::Prepare => "prepare",
            Self::Admit => "admit",
            Self::Start => "start",
            Self::Observe => "observe",
            Self::Signal => "signal",
            Self::Terminate => "terminate",
            Self::Cleanup => "cleanup",
            Self::Policy => "policy",
            Self::Io => "io",
            Self::NotFound => "not_found",
            Self::State => "state",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DriverError {
    kind: DriverErrorKind,
    message: String,
}

impl DriverError {
    pub fn new(kind: DriverErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> DriverErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for DriverError {}

impl From<std::io::Error> for DriverError {
    fn from(value: std::io::Error) -> Self {
        Self::new(DriverErrorKind::Io, value.to_string())
    }
}
