//! In-memory policy engine: deny-by-default action checks.

use std::collections::{BTreeMap, BTreeSet};

use crate::policy::action::Action;
use crate::policy::decision::{Decision, DecisionEffect};
use crate::policy::principal::PrincipalId;
use crate::policy::role::Role;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyError {
    UnknownPrincipal,
    EmptyPrincipal,
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownPrincipal => write!(f, "unknown principal"),
            Self::EmptyPrincipal => write!(f, "empty principal"),
        }
    }
}

impl std::error::Error for PolicyError {}

/// Cluster authorization policy (RAM-only revisioned matrix).
#[derive(Clone, Debug, Default)]
pub struct PolicyEngine {
    revision: u64,
    /// Explicit action grants (principal → actions).
    grants: BTreeMap<PrincipalId, BTreeSet<Action>>,
    /// Role bindings expand to actions at check time.
    roles: BTreeMap<PrincipalId, BTreeSet<Role>>,
    next_decision: u64,
}

impl PolicyEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn bump(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    pub fn bind_role(&mut self, principal: PrincipalId, role: Role) {
        self.roles.entry(principal).or_default().insert(role);
        self.bump();
    }

    pub fn grant(&mut self, principal: PrincipalId, action: Action) {
        self.grants.entry(principal).or_default().insert(action);
        self.bump();
    }

    pub fn revoke_grant(&mut self, principal: &PrincipalId, action: &Action) -> bool {
        let removed = self
            .grants
            .get_mut(principal)
            .map(|set| set.remove(action))
            .unwrap_or(false);
        if removed {
            self.bump();
        }
        removed
    }

    /// Authorize `action` for `principal`. Deny-by-default.
    pub fn authorize(&mut self, principal: &PrincipalId, action: &Action) -> Decision {
        self.next_decision = self.next_decision.saturating_add(1);
        let decision_id = format!("pd-{}", self.next_decision);
        let allowed = self.is_allowed(principal, action);
        Decision {
            effect: if allowed {
                DecisionEffect::Allow
            } else {
                DecisionEffect::Deny
            },
            decision_id,
            action: action.clone(),
            policy_revision: self.revision,
            reason: if allowed {
                "explicit_grant_or_role"
            } else {
                "deny_by_default"
            },
        }
    }

    fn is_allowed(&self, principal: &PrincipalId, action: &Action) -> bool {
        if let Some(grants) = self.grants.get(principal) {
            if grants.contains(action) || wildcard_match(grants, action) {
                return true;
            }
        }
        if let Some(roles) = self.roles.get(principal) {
            for role in roles {
                for role_action in role.actions() {
                    if &role_action == action || action_matches(&role_action, action) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

fn wildcard_match(grants: &BTreeSet<Action>, action: &Action) -> bool {
    grants.iter().any(|g| action_matches(g, action))
}

/// Role/grant wildcards use `"*"` in parameterized scopes.
fn action_matches(pattern: &Action, action: &Action) -> bool {
    match (pattern, action) {
        (Action::PublicationUse { provider: p }, Action::PublicationUse { provider: a }) => {
            p == "*" || p == a
        }
        (Action::ConnectorUse { name: p }, Action::ConnectorUse { name: a }) => p == "*" || p == a,
        (Action::HiccupPublish { topic: p }, Action::HiccupPublish { topic: a }) => {
            p == "*" || p == a
        }
        (Action::HiccupListen { topic: p }, Action::HiccupListen { topic: a }) => {
            p == "*" || p == a
        }
        _ => pattern == action,
    }
}
