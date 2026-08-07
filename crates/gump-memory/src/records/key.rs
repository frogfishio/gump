//! Typed record keyspace (PROTOCOL.md §7).

use core::fmt;

/// Known key prefixes; callers cannot invent others.
#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum KeyPrefix {
    ClusterMeta,
    Members,
    AuthorityController,
    Names,
    WorkloadsDesired,
    WorkloadsHistory,
    Executions,
    Units,
    Placements,
    Attempts,
    Barriers,
    Materializations,
    Publication,
    Custody,
    Operations,
    Observations,
    Reasons,
}

impl KeyPrefix {
    /// Maximum payload bytes for this prefix (PROTOCOL.md §7 table).
    pub const fn max_payload(self) -> usize {
        match self {
            Self::ClusterMeta | Self::Members => 64 * 1024,
            Self::AuthorityController => 8 * 1024,
            Self::Names => 4 * 1024,
            Self::WorkloadsDesired => 256 * 1024,
            Self::WorkloadsHistory => 64 * 1024,
            Self::Executions | Self::Attempts | Self::Operations | Self::Observations => 64 * 1024,
            Self::Units | Self::Placements | Self::Publication => 32 * 1024,
            Self::Barriers => 256 * 1024,
            Self::Materializations | Self::Custody => 8 * 1024,
            Self::Reasons => 64 * 1024,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClusterMeta => "/cluster/meta",
            Self::Members => "/members",
            Self::AuthorityController => "/authority/controller",
            Self::Names => "/names",
            Self::WorkloadsDesired => "/workloads/desired",
            Self::WorkloadsHistory => "/workloads/history",
            Self::Executions => "/executions",
            Self::Units => "/units",
            Self::Placements => "/placements",
            Self::Attempts => "/attempts",
            Self::Barriers => "/barriers",
            Self::Materializations => "/materializations",
            Self::Publication => "/publication",
            Self::Custody => "/custody",
            Self::Operations => "/operations",
            Self::Observations => "/observations",
            Self::Reasons => "/reasons",
        }
    }

    /// Authoritative live records count against the authoritative budget;
    /// leased-capable prefixes count as leased when a lease is attached.
    pub const fn default_class(self) -> RecordClass {
        match self {
            Self::WorkloadsHistory | Self::Operations | Self::Observations | Self::Reasons => {
                RecordClass::History
            }
            Self::Members
            | Self::AuthorityController
            | Self::Placements
            | Self::Attempts
            | Self::Barriers
            | Self::Materializations
            | Self::Publication
            | Self::Custody => RecordClass::LeasedCapable,
            _ => RecordClass::Authoritative,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum RecordClass {
    Authoritative,
    LeasedCapable,
    History,
}

/// Typed record key: closed prefix + bounded suffix.
#[derive(
    Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct RecordKey {
    pub prefix: KeyPrefix,
    pub suffix: String,
}

impl RecordKey {
    pub const MAX_SUFFIX: usize = 256;

    pub fn new(prefix: KeyPrefix, suffix: impl Into<String>) -> Result<Self, KeyError> {
        let suffix = suffix.into();
        if suffix.len() > Self::MAX_SUFFIX {
            return Err(KeyError::SuffixTooLong {
                len: suffix.len(),
                max: Self::MAX_SUFFIX,
            });
        }
        if suffix.chars().any(|c| c == '\0' || c.is_control()) {
            return Err(KeyError::InvalidSuffix);
        }
        Ok(Self { prefix, suffix })
    }
}

impl fmt::Display for RecordKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.suffix.is_empty() {
            write!(f, "{}", self.prefix.as_str())
        } else {
            write!(f, "{}/{}", self.prefix.as_str(), self.suffix)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyError {
    SuffixTooLong { len: usize, max: usize },
    InvalidSuffix,
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SuffixTooLong { len, max } => {
                write!(f, "key suffix length {len} exceeds max {max}")
            }
            Self::InvalidSuffix => write!(f, "key suffix contains invalid characters"),
        }
    }
}

impl std::error::Error for KeyError {}
