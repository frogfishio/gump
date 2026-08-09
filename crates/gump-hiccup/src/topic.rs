//! Topic validation and `@self` resolution (HICCUP.md §5).

use core::fmt;

use gump_types::WorkloadId;

use crate::limits::MAX_LISTEN_TOPICS;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TopicError {
    Empty,
    TooLong { len: usize },
    InvalidShape,
    ReservedGumpPrefix,
    ListenWithoutPublishRequiresListen,
    TooManyListen { count: usize },
    DuplicateListen,
    CrossWorkloadSelf,
}

impl fmt::Display for TopicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "topic is empty"),
            Self::TooLong { len } => write!(f, "topic length {len} exceeds 128"),
            Self::InvalidShape => {
                write!(f, "topic must be @self or lowercase slash-separated ASCII")
            }
            Self::ReservedGumpPrefix => write!(f, "gump/ topics are reserved"),
            Self::ListenWithoutPublishRequiresListen => {
                write!(f, "topic null requires non-empty listen")
            }
            Self::TooManyListen { count } => {
                write!(f, "listen has {count} topics; max {MAX_LISTEN_TOPICS}")
            }
            Self::DuplicateListen => write!(f, "listen topics must be unique"),
            Self::CrossWorkloadSelf => write!(f, "@self cannot cross workload identity"),
        }
    }
}

impl std::error::Error for TopicError {}

/// Canonical topic string stored on the board (never leading `#`).
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct CanonicalTopic(String);

impl CanonicalTopic {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Internal `@self` binding for a workload.
    pub fn self_for(workload: WorkloadId) -> Self {
        Self(format!("@self/{}", workload.to_hyphenated()))
    }
}

impl fmt::Display for CanonicalTopic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Validate a JSON topic token (`@self` or named).
pub fn validate_topic_token(raw: &str) -> Result<(), TopicError> {
    if raw == "@self" {
        return Ok(());
    }
    if raw.is_empty() || raw.len() > 128 {
        return Err(if raw.is_empty() {
            TopicError::Empty
        } else {
            TopicError::TooLong { len: raw.len() }
        });
    }
    if raw.starts_with("gump/") {
        return Err(TopicError::ReservedGumpPrefix);
    }
    let t = raw.strip_prefix('#').unwrap_or(raw);
    if t.is_empty() || t.starts_with('/') || t.ends_with('/') || t.contains("//") {
        return Err(TopicError::InvalidShape);
    }
    for seg in t.split('/') {
        if seg.is_empty() || seg.len() > 64 {
            return Err(TopicError::InvalidShape);
        }
        let bytes = seg.as_bytes();
        if !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit() {
            return Err(TopicError::InvalidShape);
        }
        if !bytes[bytes.len() - 1].is_ascii_lowercase() && !bytes[bytes.len() - 1].is_ascii_digit()
        {
            return Err(TopicError::InvalidShape);
        }
        for &b in bytes {
            if !matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-') {
                return Err(TopicError::InvalidShape);
            }
        }
    }
    Ok(())
}

pub fn canonicalize_topic(raw: &str, workload: WorkloadId) -> Result<CanonicalTopic, TopicError> {
    validate_topic_token(raw)?;
    if raw == "@self" {
        return Ok(CanonicalTopic::self_for(workload));
    }
    let t = raw.strip_prefix('#').unwrap_or(raw);
    Ok(CanonicalTopic(t.to_string()))
}

/// Resolved publish + listen set for a declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedTopics {
    /// `None` means listen-only (JSON `topic: null`).
    pub publish: Option<CanonicalTopic>,
    pub listen: Vec<CanonicalTopic>,
}

/// `topic`: `None` = omitted (default `@self`); `Some(None)` = JSON null; `Some(Some(s))` = explicit.
pub fn resolve_topics(
    topic: Option<Option<&str>>,
    listen: Option<&[String]>,
    workload: WorkloadId,
) -> Result<ResolvedTopics, TopicError> {
    let publish = match topic {
        None => Some(CanonicalTopic::self_for(workload)),
        Some(None) => None,
        Some(Some(s)) => Some(canonicalize_topic(s, workload)?),
    };

    let listen = match listen {
        None => match &publish {
            Some(p) => vec![p.clone()],
            None => return Err(TopicError::ListenWithoutPublishRequiresListen),
        },
        Some(items) => {
            if items.is_empty() && publish.is_none() {
                return Err(TopicError::ListenWithoutPublishRequiresListen);
            }
            if items.len() > MAX_LISTEN_TOPICS {
                return Err(TopicError::TooManyListen { count: items.len() });
            }
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let c = canonicalize_topic(item, workload)?;
                if out.contains(&c) {
                    return Err(TopicError::DuplicateListen);
                }
                out.push(c);
            }
            out
        }
    };

    Ok(ResolvedTopics { publish, listen })
}

/// `@self` for workload A must never equal `@self` for workload B.
pub fn assert_self_isolation(a: WorkloadId, b: WorkloadId) -> Result<(), TopicError> {
    if CanonicalTopic::self_for(a) == CanonicalTopic::self_for(b) && a != b {
        Err(TopicError::CrossWorkloadSelf)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workload() -> WorkloadId {
        WorkloadId::from_bytes([
            0x01, 0x8f, 0x4a, 0, 0, 0, 0x70, 0, 0x80, 0, 0, 0, 0, 0, 0, 1,
        ])
        .unwrap()
    }

    #[test]
    fn explicit_empty_listen_means_publish_only() {
        let empty = Vec::<String>::new();
        let resolved = resolve_topics(
            Some(Some("telemetry/sink/ratatouille-http")),
            Some(&empty),
            workload(),
        )
        .unwrap();
        assert!(resolved.publish.is_some());
        assert!(resolved.listen.is_empty());
    }

    #[test]
    fn omitted_listen_still_defaults_to_published_topic() {
        let resolved = resolve_topics(
            Some(Some("telemetry/sink/ratatouille-http")),
            None,
            workload(),
        )
        .unwrap();
        assert_eq!(resolved.listen.len(), 1);
        assert_eq!(resolved.publish.as_ref(), resolved.listen.first());
    }
}
