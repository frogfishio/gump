//! Safe, redacting error surface (SECURITY §13).

use core::fmt;

use crate::secret::Secret;

/// Stable machine-facing reason codes. Extend as protocol codes land (W03+).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ReasonCode {
    Cancelled,
    InvalidArgument,
    NotFound,
    Conflict,
    Unauthorized,
    FailedPrecondition,
    ResourceExhausted,
    Unavailable,
    Internal,
}

impl ReasonCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::InvalidArgument => "invalid_argument",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::Unauthorized => "unauthorized",
            Self::FailedPrecondition => "failed_precondition",
            Self::ResourceExhausted => "resource_exhausted",
            Self::Unavailable => "unavailable",
            Self::Internal => "internal",
        }
    }
}

impl fmt::Display for ReasonCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Operator/peer-safe error: IDs and reason codes only — never secret material.
#[derive(Clone, Eq, PartialEq)]
pub struct SafeError {
    reason: ReasonCode,
    /// Optional object id (already a public identifier string).
    object_id: Option<String>,
    /// Human-safe message; must not embed secret bytes.
    message: String,
}

impl SafeError {
    pub fn new(reason: ReasonCode, message: impl Into<String>) -> Self {
        Self {
            reason,
            object_id: None,
            message: message.into(),
        }
    }

    pub fn with_object_id(mut self, object_id: impl Into<String>) -> Self {
        self.object_id = Some(object_id.into());
        self
    }

    pub fn reason(&self) -> ReasonCode {
        self.reason
    }

    pub fn object_id(&self) -> Option<&str> {
        self.object_id.as_deref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// Attach context that may contain secrets — they are stored redacted-only
    /// via [`Secret`] and never appear in `Display`/`Debug` of this error.
    pub fn redact_context(secret: Secret<String>) -> String {
        // Intentionally discard plaintext: callers pass secrets through Secret
        // so Debug of the Secret itself is already redacted; we only keep a
        // placeholder for audit of "context was present".
        let _ = secret;
        "<redacted>".to_owned()
    }
}

impl fmt::Debug for SafeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SafeError")
            .field("reason", &self.reason)
            .field("object_id", &self.object_id)
            .field("message", &self.message)
            .finish()
    }
}

impl fmt::Display for SafeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.reason, self.message)?;
        if let Some(id) = &self.object_id {
            write!(f, " (object_id={id})")?;
        }
        Ok(())
    }
}

impl std::error::Error for SafeError {}

impl From<crate::cancel::Cancelled> for SafeError {
    fn from(_: crate::cancel::Cancelled) -> Self {
        Self::new(ReasonCode::Cancelled, "operation cancelled")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::Secret;

    #[test]
    fn safe_error_debug_has_no_secret_bytes() {
        let secret = Secret::new("super-secret-token".to_owned());
        let placeholder = SafeError::redact_context(secret);
        let err = SafeError::new(ReasonCode::Unauthorized, placeholder)
            .with_object_id("01900000-0000-7000-8000-000000000001");
        let rendered = format!("{err:?}");
        assert!(!rendered.contains("super-secret-token"));
        assert!(rendered.contains("unauthorized") || rendered.contains("Unauthorized"));
    }
}
