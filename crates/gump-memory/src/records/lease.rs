//! Leases (PROTOCOL.md §8).

use std::collections::BTreeMap;

/// Default lease TTLs from PROTOCOL.md §8 (milliseconds).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeasePurpose {
    ControllerAuthority,
    MemberLiveness,
    PlacementAttempt,
    GangReservation,
    TelemetrySubscription,
}

impl LeasePurpose {
    pub const fn ttl_ms(self) -> u64 {
        match self {
            Self::ControllerAuthority => 10_000,
            Self::MemberLiveness => 15_000,
            Self::PlacementAttempt => 20_000,
            Self::GangReservation => 30_000,
            Self::TelemetrySubscription => 30_000,
        }
    }

    pub const fn renew_by_ms(self) -> u64 {
        match self {
            Self::ControllerAuthority => 3_000,
            Self::MemberLiveness => 5_000,
            Self::PlacementAttempt => 6_000,
            Self::GangReservation => 10_000,
            Self::TelemetrySubscription => 10_000,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Lease {
    pub id: u64,
    pub purpose: LeasePurpose,
    pub ttl_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, Default)]
pub struct LeaseTable {
    next_id: u64,
    leases: BTreeMap<u64, Lease>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LeaseError {
    NotFound(u64),
    Expired(u64),
}

impl std::fmt::Display for LeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "lease {id} not found"),
            Self::Expired(id) => write!(f, "lease {id} expired"),
        }
    }
}

impl std::error::Error for LeaseError {}

impl LeaseTable {
    pub fn get(&self, id: u64) -> Option<&Lease> {
        self.leases.get(&id)
    }

    pub fn len(&self) -> usize {
        self.leases.len()
    }

    pub fn is_empty(&self) -> bool {
        self.leases.is_empty()
    }

    pub fn grant(&mut self, purpose: LeasePurpose, now_ms: u64) -> Lease {
        self.next_id = self.next_id.saturating_add(1);
        let id = self.next_id;
        let ttl_ms = purpose.ttl_ms();
        let lease = Lease {
            id,
            purpose,
            ttl_ms,
            expires_at_ms: now_ms.saturating_add(ttl_ms),
        };
        self.leases.insert(id, lease.clone());
        lease
    }

    /// Renew without extending beyond one original TTL from now (PROTOCOL failover bound).
    pub fn renew(&mut self, id: u64, now_ms: u64) -> Result<Lease, LeaseError> {
        let lease = self.leases.get_mut(&id).ok_or(LeaseError::NotFound(id))?;
        if lease.expires_at_ms <= now_ms {
            self.leases.remove(&id);
            return Err(LeaseError::Expired(id));
        }
        lease.expires_at_ms = now_ms.saturating_add(lease.ttl_ms);
        Ok(lease.clone())
    }

    pub fn revoke(&mut self, id: u64) -> Result<Lease, LeaseError> {
        self.leases.remove(&id).ok_or(LeaseError::NotFound(id))
    }

    /// Remove leases with `expires_at_ms <= now_ms`. Returns revoked ids.
    pub fn expire_due(&mut self, now_ms: u64) -> Vec<u64> {
        let due: Vec<u64> = self
            .leases
            .iter()
            .filter(|(_, l)| l.expires_at_ms <= now_ms)
            .map(|(id, _)| *id)
            .collect();
        for id in &due {
            self.leases.remove(id);
        }
        due
    }
}
