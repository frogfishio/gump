//! Node transport identity bound into certificates (PROTOCOL.md §3).

use core::fmt;

use std::str::FromStr;

use gump_types::{ClusterId, IncarnationId, NodeId};

/// Roles advertised on a transport certificate.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum NodeRole {
    Memory,
    Agent,
    Controller,
    Ingress,
}

impl NodeRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Agent => "agent",
            Self::Controller => "controller",
            Self::Ingress => "ingress",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "memory" => Some(Self::Memory),
            "agent" => Some(Self::Agent),
            "controller" => Some(Self::Controller),
            "ingress" => Some(Self::Ingress),
            _ => None,
        }
    }
}

/// Authenticated peer identity after mTLS (necessary, never sufficient authz).
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TransportIdentity {
    pub cluster_id: ClusterId,
    pub node_id: NodeId,
    pub incarnation: IncarnationId,
    pub roles: Vec<NodeRole>,
}

impl TransportIdentity {
    /// Encode into a stable DNS-like SAN label set used by test/local material.
    ///
    /// Format: `gump.node.<node>/gump.cluster.<cluster>/gump.inc.<incarnation>`
    /// plus `gump.role.<role>` for each role.
    pub fn san_names(&self) -> Vec<String> {
        let mut names = vec![
            "localhost".to_string(),
            format!("gump.node.{}", self.node_id.to_hyphenated()),
            format!("gump.cluster.{}", self.cluster_id.to_hyphenated()),
            format!("gump.inc.{}", self.incarnation.to_hyphenated()),
        ];
        for role in &self.roles {
            names.push(format!("gump.role.{}", role.as_str()));
        }
        names
    }

    /// Parse identity from certificate DNS names (inverse of [`Self::san_names`]).
    pub fn from_san_names(names: &[String]) -> Result<Self, IdentityParseError> {
        let mut cluster = None;
        let mut node = None;
        let mut incarnation = None;
        let mut roles = Vec::new();
        for name in names {
            if let Some(rest) = name.strip_prefix("gump.cluster.") {
                cluster = Some(
                    ClusterId::from_str(rest)
                        .map_err(|_| IdentityParseError::BadUuid { field: "cluster" })?,
                );
            } else if let Some(rest) = name.strip_prefix("gump.node.") {
                node = Some(
                    NodeId::from_str(rest)
                        .map_err(|_| IdentityParseError::BadUuid { field: "node" })?,
                );
            } else if let Some(rest) = name.strip_prefix("gump.inc.") {
                incarnation = Some(IncarnationId::from_str(rest).map_err(|_| {
                    IdentityParseError::BadUuid {
                        field: "incarnation",
                    }
                })?);
            } else if let Some(rest) = name.strip_prefix("gump.role.") {
                let role = NodeRole::parse(rest).ok_or(IdentityParseError::BadRole)?;
                roles.push(role);
            }
        }
        roles.sort();
        roles.dedup();
        Ok(Self {
            cluster_id: cluster.ok_or(IdentityParseError::MissingField("cluster"))?,
            node_id: node.ok_or(IdentityParseError::MissingField("node"))?,
            incarnation: incarnation.ok_or(IdentityParseError::MissingField("incarnation"))?,
            roles,
        })
    }

    /// Duplicate-session selector key (PROTOCOL.md §3).
    pub fn session_key(&self, connection_nonce: &[u8; 16]) -> (NodeId, [u8; 16]) {
        (self.node_id, *connection_nonce)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IdentityParseError {
    MissingField(&'static str),
    BadUuid { field: &'static str },
    BadRole,
}

impl fmt::Display for IdentityParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField(field) => write!(f, "missing identity field {field}"),
            Self::BadUuid { field } => write!(f, "invalid UUID for {field}"),
            Self::BadRole => write!(f, "unknown node role"),
        }
    }
}

impl std::error::Error for IdentityParseError {}

/// Prefer the session from the lexicographically smaller `(node_id, nonce)`.
pub fn prefer_session(a: &(NodeId, [u8; 16]), b: &(NodeId, [u8; 16])) -> OrderingPrefer {
    match a.cmp(b) {
        core::cmp::Ordering::Less => OrderingPrefer::KeepA,
        core::cmp::Ordering::Greater => OrderingPrefer::KeepB,
        core::cmp::Ordering::Equal => OrderingPrefer::KeepA,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrderingPrefer {
    KeepA,
    KeepB,
}
