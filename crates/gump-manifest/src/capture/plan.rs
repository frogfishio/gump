//! Capture plan from a normalized `Package` section.

use core::fmt;

use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::model::Package;

use super::deny::SensitiveDeny;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CaptureErrorKind {
    Glob,
    Escape,
    Sensitive,
    Io,
    SourceChanged,
    Policy,
    Prepare,
}

impl fmt::Display for CaptureErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Glob => "glob",
            Self::Escape => "escape",
            Self::Sensitive => "sensitive",
            Self::Io => "io",
            Self::SourceChanged => "source_changed",
            Self::Policy => "policy",
            Self::Prepare => "prepare",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureError {
    kind: CaptureErrorKind,
    message: String,
}

impl CaptureError {
    pub fn new(kind: CaptureErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> CaptureErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for CaptureError {}

impl From<SensitiveDeny> for CaptureError {
    fn from(value: SensitiveDeny) -> Self {
        let kind = match value {
            SensitiveDeny::Escape | SensitiveDeny::Absolute | SensitiveDeny::Empty => {
                CaptureErrorKind::Escape
            }
            _ => CaptureErrorKind::Sensitive,
        };
        Self::new(kind, value.as_str())
    }
}

impl From<std::io::Error> for CaptureError {
    fn from(value: std::io::Error) -> Self {
        Self::new(CaptureErrorKind::Io, value.to_string())
    }
}

/// Compiled include/exclude plan for one package root.
#[derive(Clone, Debug)]
pub struct CapturePlan {
    pub root_pattern: String,
    pub allow_workspace_root: bool,
    pub allow_sensitive_files: bool,
    include: GlobSet,
    exclude: GlobSet,
}

impl CapturePlan {
    pub fn from_package(package: &Package) -> Result<Self, CaptureError> {
        if package.include.iter().any(|p| p == "." || p == "**") && !package.allow_workspace_root {
            return Err(CaptureError::new(
                CaptureErrorKind::Policy,
                "include of workspace root requires package.allow_workspace_root=true",
            ));
        }
        Ok(Self {
            root_pattern: package.root.clone(),
            allow_workspace_root: package.allow_workspace_root,
            allow_sensitive_files: package.allow_sensitive_files,
            include: build_globs(&package.include)?,
            exclude: build_globs(&package.exclude)?,
        })
    }

    pub fn matches(&self, rel: &str) -> bool {
        if !self.include.is_match(rel) {
            return false;
        }
        if self.exclude.is_match(rel) {
            return false;
        }
        true
    }
}

fn build_globs(patterns: &[String]) -> Result<GlobSet, CaptureError> {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let glob = Glob::new(pat).map_err(|e| {
            CaptureError::new(CaptureErrorKind::Glob, format!("invalid glob {pat:?}: {e}"))
        })?;
        builder.add(glob);
    }
    builder.build().map_err(|e| {
        CaptureError::new(
            CaptureErrorKind::Glob,
            format!("glob set build failed: {e}"),
        )
    })
}
