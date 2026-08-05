//! Typed record state machine (DELIVERY C04–C05 / PROTOCOL.md §6–§8).

use std::collections::BTreeMap;

use crate::records::budgets::{BudgetClass, BudgetError, BudgetUsage, MemoryBudgets};
use crate::records::command::{Command, Comparison, Expected, MutateOp, Txn};
use crate::records::key::{KeyError, KeyPrefix, RecordClass, RecordKey};
use crate::records::lease::{Lease, LeaseError, LeaseTable};
use crate::records::value::{RecordValue, ValueError};
use crate::records::watch::{Compacted, WatchBatch, WatchChange, WatchHistory};

#[derive(Clone, Debug)]
pub struct TypedRecordMachine {
    revision: u64,
    records: BTreeMap<RecordKey, RecordValue>,
    budgets: MemoryBudgets,
    usage: BudgetUsage,
    /// Monotonic leader clock (milliseconds) for leases and watch age.
    now_ms: u64,
    watch: WatchHistory,
    leases: LeaseTable,
    /// Pending changes for the current revision batch (txn coalescing).
    pending: Vec<WatchChange>,
}

impl Default for TypedRecordMachine {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplyError {
    Key(KeyError),
    Value(ValueError),
    Budget(BudgetError),
    Lease(LeaseError),
    PreconditionFailed {
        key: RecordKey,
        expected: Expected,
    },
    NotFound(RecordKey),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Key(e) => write!(f, "{e}"),
            Self::Value(e) => write!(f, "{e}"),
            Self::Budget(e) => write!(f, "{e}"),
            Self::Lease(e) => write!(f, "{e}"),
            Self::PreconditionFailed { key, expected } => {
                write!(f, "precondition failed for {key}: {expected:?}")
            }
            Self::NotFound(key) => write!(f, "key not found: {key}"),
        }
    }
}

impl std::error::Error for ApplyError {}

impl From<KeyError> for ApplyError {
    fn from(e: KeyError) -> Self {
        Self::Key(e)
    }
}

impl From<ValueError> for ApplyError {
    fn from(e: ValueError) -> Self {
        Self::Value(e)
    }
}

impl From<BudgetError> for ApplyError {
    fn from(e: BudgetError) -> Self {
        Self::Budget(e)
    }
}

impl From<LeaseError> for ApplyError {
    fn from(e: LeaseError) -> Self {
        Self::Lease(e)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyResult {
    pub revision: u64,
    pub txn_succeeded: Option<bool>,
    pub lease: Option<Lease>,
    pub expired_lease_ids: Vec<u64>,
}

impl TypedRecordMachine {
    pub fn new(budgets: MemoryBudgets) -> Self {
        Self {
            revision: 0,
            records: BTreeMap::new(),
            budgets,
            usage: BudgetUsage::default(),
            now_ms: 0,
            watch: WatchHistory::default(),
            leases: LeaseTable::default(),
            pending: Vec::new(),
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(MemoryBudgets::default())
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn usage(&self) -> BudgetUsage {
        self.usage
    }

    pub fn budgets(&self) -> MemoryBudgets {
        self.budgets
    }

    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }

    /// Advance or set the monotonic clock (simulation / leader tick).
    pub fn set_now_ms(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
        self.watch.retain(self.now_ms, self.budgets.history_bytes);
    }

    pub fn advance_now_ms(&mut self, by_ms: u64) {
        self.set_now_ms(self.now_ms.saturating_add(by_ms));
    }

    pub fn compaction_floor(&self) -> u64 {
        self.watch.floor()
    }

    pub fn get(&self, key: &RecordKey) -> Option<&RecordValue> {
        self.records.get(key)
    }

    pub fn get_lease(&self, id: u64) -> Option<&Lease> {
        self.leases.get(id)
    }

    /// Watches start strictly after `after`; lagging watchers get COMPACTED.
    pub fn watch_after(&self, after: u64) -> Result<Vec<WatchBatch>, Compacted> {
        self.watch.watch_after(after)
    }

    pub fn apply(&mut self, cmd: Command) -> Result<ApplyResult, ApplyError> {
        match cmd {
            Command::Put {
                key,
                expected,
                payload,
                leased,
            } => {
                self.apply_mutate(MutateOp::Put {
                    key,
                    expected,
                    payload,
                    leased,
                })?;
                self.flush_pending();
                Ok(ApplyResult {
                    revision: self.revision,
                    txn_succeeded: None,
                    lease: None,
                    expired_lease_ids: Vec::new(),
                })
            }
            Command::Delete { key, expected } => {
                self.apply_mutate(MutateOp::Delete { key, expected })?;
                self.flush_pending();
                Ok(ApplyResult {
                    revision: self.revision,
                    txn_succeeded: None,
                    lease: None,
                    expired_lease_ids: Vec::new(),
                })
            }
            Command::Txn(txn) => self.apply_txn(txn),
            Command::Compact { through } => {
                self.watch.compact_through(through);
                Ok(ApplyResult {
                    revision: self.revision,
                    txn_succeeded: None,
                    lease: None,
                    expired_lease_ids: Vec::new(),
                })
            }
            Command::LeaseGrant { purpose } => {
                let lease = self.leases.grant(purpose, self.now_ms);
                self.revision = self.revision.saturating_add(1);
                Ok(ApplyResult {
                    revision: self.revision,
                    txn_succeeded: None,
                    lease: Some(lease),
                    expired_lease_ids: Vec::new(),
                })
            }
            Command::LeaseRenew { lease_id } => {
                let lease = self.leases.renew(lease_id, self.now_ms)?;
                self.revision = self.revision.saturating_add(1);
                Ok(ApplyResult {
                    revision: self.revision,
                    txn_succeeded: None,
                    lease: Some(lease),
                    expired_lease_ids: Vec::new(),
                })
            }
            Command::LeaseRevoke { lease_id } => {
                self.leases.revoke(lease_id)?;
                self.revision = self.revision.saturating_add(1);
                self.pending.push(WatchChange::LeaseRevoked {
                    lease_id,
                    revision: self.revision,
                });
                self.flush_pending();
                Ok(ApplyResult {
                    revision: self.revision,
                    txn_succeeded: None,
                    lease: None,
                    expired_lease_ids: vec![lease_id],
                })
            }
            Command::ExpireLeases => {
                let expired = self.leases.expire_due(self.now_ms);
                if expired.is_empty() {
                    return Ok(ApplyResult {
                        revision: self.revision,
                        txn_succeeded: None,
                        lease: None,
                        expired_lease_ids: Vec::new(),
                    });
                }
                self.revision = self.revision.saturating_add(1);
                for lease_id in &expired {
                    self.pending.push(WatchChange::LeaseRevoked {
                        lease_id: *lease_id,
                        revision: self.revision,
                    });
                }
                self.flush_pending();
                Ok(ApplyResult {
                    revision: self.revision,
                    txn_succeeded: None,
                    lease: None,
                    expired_lease_ids: expired,
                })
            }
        }
    }

    fn apply_txn(&mut self, txn: Txn) -> Result<ApplyResult, ApplyError> {
        let ok = txn
            .comparisons
            .iter()
            .all(|c| self.check_expected(&c.key, &c.expected).is_ok());
        let ops = if ok {
            txn.success_ops
        } else {
            txn.failure_ops
        };
        let snap = self.snapshot();
        for op in ops {
            if let Err(e) = self.apply_mutate(op) {
                self.restore(snap);
                return Err(e);
            }
        }
        self.flush_pending();
        Ok(ApplyResult {
            revision: self.revision,
            txn_succeeded: Some(ok),
            lease: None,
            expired_lease_ids: Vec::new(),
        })
    }

    fn snapshot(&self) -> MachineSnap {
        MachineSnap {
            revision: self.revision,
            records: self.records.clone(),
            usage: self.usage,
            watch: self.watch.clone(),
            leases: self.leases.clone(),
            pending: self.pending.clone(),
        }
    }

    fn restore(&mut self, snap: MachineSnap) {
        self.revision = snap.revision;
        self.records = snap.records;
        self.usage = snap.usage;
        self.watch = snap.watch;
        self.leases = snap.leases;
        self.pending = snap.pending;
    }

    fn apply_mutate(&mut self, op: MutateOp) -> Result<(), ApplyError> {
        match op {
            MutateOp::Put {
                key,
                expected,
                payload,
                leased,
            } => self.put(key, expected, payload, leased),
            MutateOp::Delete { key, expected } => self.delete(key, expected),
        }
    }

    fn put(
        &mut self,
        key: RecordKey,
        expected: Expected,
        payload: Vec<u8>,
        leased: bool,
    ) -> Result<(), ApplyError> {
        self.check_expected(&key, &expected)?;
        let value = RecordValue::new(self.revision.saturating_add(1), payload, leased);
        value.validate_size(key.prefix)?;

        let class = budget_class(key.prefix, leased);
        let new_bytes = value.byte_len();
        let old_bytes = self
            .records
            .get(&key)
            .map(|v| (v.byte_len(), budget_class(key.prefix, v.leased)));

        let mut probe = self.usage;
        // History usage is managed via watch; keep record budgets separate.
        if let Some((old, old_class)) = old_bytes {
            probe.sub(old_class, old);
        }
        probe.check_grow(&self.budgets, class, new_bytes)?;

        if let Some((old, old_class)) = old_bytes {
            self.usage.sub(old_class, old);
        }
        self.usage.add(class, new_bytes);
        self.revision = value.revision;
        self.pending.push(WatchChange::Put {
            key: key.clone(),
            revision: value.revision,
            digest: value.digest,
        });
        self.records.insert(key, value);
        Ok(())
    }

    fn delete(&mut self, key: RecordKey, expected: Expected) -> Result<(), ApplyError> {
        self.check_expected(&key, &expected)?;
        let Some(old) = self.records.remove(&key) else {
            return Err(ApplyError::NotFound(key));
        };
        let class = budget_class(key.prefix, old.leased);
        self.usage.sub(class, old.byte_len());
        self.revision = self.revision.saturating_add(1);
        self.pending.push(WatchChange::Delete {
            key,
            revision: self.revision,
        });
        Ok(())
    }

    fn flush_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let changes = std::mem::take(&mut self.pending);
        let batch = WatchBatch {
            revision: self.revision,
            at_ms: self.now_ms,
            changes,
        };
        self.watch.push(batch, self.now_ms, self.budgets.history_bytes);
    }

    fn check_expected(&self, key: &RecordKey, expected: &Expected) -> Result<(), ApplyError> {
        let cur = self.records.get(key);
        let ok = match expected {
            Expected::Any => true,
            Expected::Absent => cur.is_none(),
            Expected::ExactRevision(rev) => cur.map(|v| v.revision == *rev).unwrap_or(false),
            Expected::ExactDigest(d) => cur.map(|v| &v.digest == d).unwrap_or(false),
        };
        if ok {
            Ok(())
        } else {
            Err(ApplyError::PreconditionFailed {
                key: key.clone(),
                expected: expected.clone(),
            })
        }
    }
}

struct MachineSnap {
    revision: u64,
    records: BTreeMap<RecordKey, RecordValue>,
    usage: BudgetUsage,
    watch: WatchHistory,
    leases: LeaseTable,
    pending: Vec<WatchChange>,
}

fn budget_class(prefix: KeyPrefix, leased: bool) -> BudgetClass {
    match prefix.default_class() {
        RecordClass::History => BudgetClass::History,
        RecordClass::Authoritative => BudgetClass::Authoritative,
        RecordClass::LeasedCapable => {
            if leased {
                BudgetClass::Leased
            } else {
                BudgetClass::Authoritative
            }
        }
    }
}

/// Helper for property tests: evaluate comparisons without mutating.
pub fn comparisons_hold(machine: &TypedRecordMachine, comps: &[Comparison]) -> bool {
    comps
        .iter()
        .all(|c| machine.check_expected(&c.key, &c.expected).is_ok())
}
