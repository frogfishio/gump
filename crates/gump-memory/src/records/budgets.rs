//! Memory budgets (PROTOCOL.md §7).

use core::fmt;

/// Initial budgets: 64 MiB authoritative, 32 MiB leased, 32 MiB history.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryBudgets {
    pub authoritative_bytes: u64,
    pub leased_bytes: u64,
    pub history_bytes: u64,
}

impl Default for MemoryBudgets {
    fn default() -> Self {
        Self {
            authoritative_bytes: 64 * 1024 * 1024,
            leased_bytes: 32 * 1024 * 1024,
            history_bytes: 32 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BudgetUsage {
    pub authoritative_bytes: u64,
    pub leased_bytes: u64,
    pub history_bytes: u64,
}

impl BudgetUsage {
    pub fn check_grow(
        &self,
        budgets: &MemoryBudgets,
        class: BudgetClass,
        additional: u64,
    ) -> Result<(), BudgetError> {
        let (used, limit) = match class {
            BudgetClass::Authoritative => (self.authoritative_bytes, budgets.authoritative_bytes),
            BudgetClass::Leased => (self.leased_bytes, budgets.leased_bytes),
            BudgetClass::History => (self.history_bytes, budgets.history_bytes),
        };
        let next = used.saturating_add(additional);
        if next > limit {
            return Err(BudgetError::Exhausted {
                class,
                used,
                additional,
                limit,
            });
        }
        Ok(())
    }

    pub fn add(&mut self, class: BudgetClass, bytes: u64) {
        match class {
            BudgetClass::Authoritative => self.authoritative_bytes += bytes,
            BudgetClass::Leased => self.leased_bytes += bytes,
            BudgetClass::History => self.history_bytes += bytes,
        }
    }

    pub fn sub(&mut self, class: BudgetClass, bytes: u64) {
        match class {
            BudgetClass::Authoritative => {
                self.authoritative_bytes = self.authoritative_bytes.saturating_sub(bytes);
            }
            BudgetClass::Leased => {
                self.leased_bytes = self.leased_bytes.saturating_sub(bytes);
            }
            BudgetClass::History => {
                self.history_bytes = self.history_bytes.saturating_sub(bytes);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetClass {
    Authoritative,
    Leased,
    History,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetError {
    Exhausted {
        class: BudgetClass,
        used: u64,
        additional: u64,
        limit: u64,
    },
}

impl fmt::Display for BudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exhausted {
                class,
                used,
                additional,
                limit,
            } => write!(
                f,
                "{class:?} budget exhausted: used={used} +{additional} > limit={limit}"
            ),
        }
    }
}

impl std::error::Error for BudgetError {}
