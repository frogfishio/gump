//! Standard sensitive-path denial (FORMATS.md §11 / D009).

use std::path::{Component, Path};

/// Why a path was denied by the structural scanner.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SensitiveDeny {
    DotGit,
    DotGump,
    LocalToml,
    DotEnv,
    PrivateKeyExt,
    CredentialName,
    EditorJunk,
    Escape,
    Absolute,
    Empty,
}

impl SensitiveDeny {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DotGit => "dot_git",
            Self::DotGump => "dot_gump",
            Self::LocalToml => "gump_local_toml",
            Self::DotEnv => "dot_env",
            Self::PrivateKeyExt => "private_key_ext",
            Self::CredentialName => "credential_name",
            Self::EditorJunk => "editor_junk",
            Self::Escape => "path_escape",
            Self::Absolute => "absolute_path",
            Self::Empty => "empty_path",
        }
    }
}

/// Normalize and classify a relative package path. Rejects escapes.
pub fn is_sensitive_relative_path(rel: &str) -> Result<Option<SensitiveDeny>, SensitiveDeny> {
    if rel.is_empty() {
        return Err(SensitiveDeny::Empty);
    }
    let path = Path::new(rel);
    if path.is_absolute() {
        return Err(SensitiveDeny::Absolute);
    }
    let mut normalized = Vec::new();
    for comp in path.components() {
        match comp {
            Component::Normal(s) => normalized.push(s.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir => return Err(SensitiveDeny::Escape),
            Component::RootDir | Component::Prefix(_) => return Err(SensitiveDeny::Absolute),
        }
    }
    if normalized.is_empty() {
        return Err(SensitiveDeny::Empty);
    }

    if normalized.iter().any(|c| c == ".git") {
        return Ok(Some(SensitiveDeny::DotGit));
    }
    if normalized.iter().any(|c| c == ".gump") {
        return Ok(Some(SensitiveDeny::DotGump));
    }

    let file = normalized.last().map(|s| s.as_str()).unwrap_or("");
    let lower = file.to_ascii_lowercase();

    if file == "gump.local.toml" {
        return Ok(Some(SensitiveDeny::LocalToml));
    }
    if file.starts_with(".env") {
        return Ok(Some(SensitiveDeny::DotEnv));
    }
    if lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower.ends_with(".p12")
        || lower.ends_with(".pfx")
        || lower.ends_with(".jks")
    {
        return Ok(Some(SensitiveDeny::PrivateKeyExt));
    }
    if lower == "credentials"
        || lower == "credentials.json"
        || lower == "id_rsa"
        || lower == "id_ed25519"
        || lower.ends_with("_rsa")
    {
        return Ok(Some(SensitiveDeny::CredentialName));
    }
    if lower.ends_with('~')
        || lower.ends_with(".swp")
        || lower.ends_with(".swo")
        || (lower.starts_with('.') && lower.ends_with(".swp"))
        || lower.ends_with(".bak")
    {
        return Ok(Some(SensitiveDeny::EditorJunk));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_standard_denies() {
        assert_eq!(
            is_sensitive_relative_path(".git/config").unwrap(),
            Some(SensitiveDeny::DotGit)
        );
        assert_eq!(
            is_sensitive_relative_path(".env.prod").unwrap(),
            Some(SensitiveDeny::DotEnv)
        );
        assert_eq!(
            is_sensitive_relative_path("certs/server.pem").unwrap(),
            Some(SensitiveDeny::PrivateKeyExt)
        );
        assert!(is_sensitive_relative_path("bin/hello").unwrap().is_none());
    }

    #[test]
    fn rejects_escape() {
        assert_eq!(
            is_sensitive_relative_path("../etc/passwd").unwrap_err(),
            SensitiveDeny::Escape
        );
    }
}
