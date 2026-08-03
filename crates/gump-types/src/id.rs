//! Typed UUIDv7 identifiers (DECISIONS D002).

use core::fmt;
use std::str::FromStr;
use uuid::Uuid;

/// Errors when parsing or validating a Gump ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdError {
    InvalidUuid,
    NotVersion7,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUuid => write!(f, "invalid UUID string"),
            Self::NotVersion7 => write!(f, "UUID is not version 7"),
        }
    }
}

impl std::error::Error for IdError {}

fn require_v7(uuid: Uuid) -> Result<Uuid, IdError> {
    if uuid.get_version() != Some(uuid::Version::SortRand) {
        // uuid crate: Version::SortRand is v7
        return Err(IdError::NotVersion7);
    }
    Ok(uuid)
}

macro_rules! gump_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
        pub struct $name(Uuid);

        impl $name {
            /// Allocate a new UUIDv7 using the process wall clock.
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// Construct from a validated UUIDv7.
            pub fn from_uuid(uuid: Uuid) -> Result<Self, IdError> {
                Ok(Self(require_v7(uuid)?))
            }

            /// Construct from the 16-byte wire encoding.
            pub fn from_bytes(bytes: [u8; 16]) -> Result<Self, IdError> {
                Self::from_uuid(Uuid::from_bytes(bytes))
            }

            pub fn as_uuid(&self) -> Uuid {
                self.0
            }

            pub fn as_bytes(&self) -> &[u8; 16] {
                self.0.as_bytes()
            }

            /// Lowercase hyphenated form for humans.
            pub fn to_hyphenated(&self) -> String {
                self.0.hyphenated().to_string()
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_tuple(stringify!($name))
                    .field(&self.to_hyphenated())
                    .finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.to_hyphenated())
            }
        }

        impl FromStr for $name {
            type Err = IdError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let uuid = Uuid::parse_str(s).map_err(|_| IdError::InvalidUuid)?;
                Self::from_uuid(uuid)
            }
        }
    };
}

gump_id!(
    /// Cluster identity.
    ClusterId
);
gump_id!(
    /// Cluster incarnation.
    IncarnationId
);
gump_id!(
    /// Node identity.
    NodeId
);
gump_id!(
    /// Capsule identity.
    CapsuleId
);
gump_id!(
    /// Workload identity (stable across releases).
    WorkloadId
);
gump_id!(
    /// Execution identity.
    ExecutionId
);
gump_id!(
    /// Placement unit identity.
    UnitId
);
gump_id!(
    /// Attempt identity.
    AttemptId
);
gump_id!(
    /// Placement-group identity.
    PlacementGroupId
);
gump_id!(
    /// Operation identity.
    OperationId
);
gump_id!(
    /// Protocol message identity.
    MessageId
);
gump_id!(
    /// Lease identity.
    LeaseId
);

/// Marker trait for documentation / generic helpers over typed IDs.
pub trait GumpId: Copy + Eq + fmt::Display + fmt::Debug {
    fn as_uuid(&self) -> Uuid;
}

macro_rules! impl_gump_id {
    ($($name:ident),+ $(,)?) => {$(
        impl GumpId for $name {
            fn as_uuid(&self) -> Uuid {
                self.0
            }
        }
    )+};
}

impl_gump_id!(
    ClusterId,
    IncarnationId,
    NodeId,
    CapsuleId,
    WorkloadId,
    ExecutionId,
    UnitId,
    AttemptId,
    PlacementGroupId,
    OperationId,
    MessageId,
    LeaseId,
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_hyphenated() {
        let id = ClusterId::new();
        let parsed: ClusterId = id.to_hyphenated().parse().unwrap();
        assert_eq!(id, parsed);
        assert_eq!(id.as_bytes().len(), 16);
    }

    #[test]
    fn rejects_non_v7() {
        let v4 = Uuid::nil();
        assert_eq!(ClusterId::from_uuid(v4), Err(IdError::NotVersion7));
    }

    #[test]
    fn ids_are_distinct_types() {
        let c = ClusterId::new();
        let n = NodeId::from_bytes(*c.as_bytes()).unwrap();
        // Same bytes, different newtypes — intentional for type safety.
        assert_eq!(c.as_bytes(), n.as_bytes());
    }
}
