//! Typed record state machine (DELIVERY C04 / PROTOCOL.md §6–§7).

use std::collections::BTreeMap;

use crate::records::budgets::{BudgetClass, BudgetError, BudgetUsage, MemoryBudgets};
use crate::records::command::{Command, Comparison, Expected, MutateOp, Txn};
use crate::records::key::{KeyError, KeyPrefix, RecordClass, RecordKey};
use crate::records::value::{RecordValue, ValueError};

#[derive(Clone, Debug, Default)]
pub struct TypedRecordMachine {
    revision: u64,
    records: BTreeMap<RecordKey, RecordValue>,
    budgets: MemoryBudgets,
    usage: BudgetUsage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplyError {
    Key(KeyError),
    Value(ValueError),
    Budget(BudgetError),
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyResult {
    pub revision: u64,
    pub txn_succeeded: Option<bool>,
}

impl TypedRecordMachine {
    pub fn new(budgets: MemoryBudgets) -> Self {
        Self {
            revision: 0,
            records: BTreeMap::new(),
            budgets,
            usage: BudgetUsage::default(),
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

    pub fn get(&self, key: &RecordKey) -> Option<&RecordValue> {
        self.records.get(key)
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
                Ok(ApplyResult {
                    revision: self.revision,
                    txn_succeeded: None,
                })
            }
            Command::Delete { key, expected } => {
                self.apply_mutate(MutateOp::Delete { key, expected })?;
                Ok(ApplyResult {
                    revision: self.revision,
                    txn_succeeded: None,
                })
            }
            Command::Txn(txn) => self.apply_txn(txn),
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
        // Snapshot for atomicity: apply all or restore.
        let snap_rev = self.revision;
        let snap_records = self.records.clone();
        let snap_usage = self.usage;
        for op in ops {
            if let Err(e) = self.apply_mutate(op) {
                self.revision = snap_rev;
                self.records = snap_records;
                self.usage = snap_usage;
                return Err(e);
            }
        }
        Ok(ApplyResult {
            revision: self.revision,
            txn_succeeded: Some(ok),
        })
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
        let old_bytes = self.records.get(&key).map(|v| (v.byte_len(), budget_class(key.prefix, v.leased)));

        // Check growth against budgets (net of replacement).
        let mut probe = self.usage;
        if let Some((old, old_class)) = old_bytes {
            probe.sub(old_class, old);
        }
        probe.check_grow(&self.budgets, class, new_bytes)?;

        // Commit.
        if let Some((old, old_class)) = old_bytes {
            self.usage.sub(old_class, old);
        }
        self.usage.add(class, new_bytes);
        self.revision = value.revision;
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
        Ok(())
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
