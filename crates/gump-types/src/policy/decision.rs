//! Authorization decision records.

use crate::policy::action::Action;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionEffect {
    Allow,
    Deny,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Decision {
    pub effect: DecisionEffect,
    pub decision_id: String,
    pub action: Action,
    pub policy_revision: u64,
    pub reason: &'static str,
}

impl Decision {
    pub fn allowed(&self) -> bool {
        self.effect == DecisionEffect::Allow
    }
}
