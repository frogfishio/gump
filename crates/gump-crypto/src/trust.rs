//! Release signer enrollment and trust policy (S02 / SECURITY.md §4).
//!
//! Authorized Ed25519 public keys are held in memory with namespace scopes,
//! optional expiry, and capability constraints. An embedded Capsule public key
//! proves a signature but grants no trust by itself.

use std::collections::{BTreeMap, BTreeSet};

use crate::fingerprint::ed25519_fingerprint;
use crate::sign::VerifyingKeyBytes;

/// Trust check purpose (ingress publication vs declaration acceptance).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustCheck {
    /// Before object publication (SECURITY §4).
    Publication,
    /// Before declaration acceptance (SECURITY §4).
    Declaration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignerEnrollment {
    pub public_key: VerifyingKeyBytes,
    /// Namespace scopes; `"*"` means all namespaces.
    pub namespaces: BTreeSet<String>,
    /// Optional absolute expiry (monotonic ms); `None` means no expiry.
    pub expires_at_ms: Option<u64>,
    /// Optional capability constraints (empty = unrestricted within scopes).
    pub capabilities: BTreeSet<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SignerRecord {
    enrollment: SignerEnrollment,
    fingerprint: String,
    revoked: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TrustError {
    NotEnrolled {
        fingerprint: String,
    },
    Revoked {
        fingerprint: String,
    },
    Expired {
        fingerprint: String,
    },
    NamespaceDenied {
        fingerprint: String,
        namespace: String,
    },
    CapabilityDenied {
        fingerprint: String,
        capability: String,
    },
    AlreadyEnrolled {
        fingerprint: String,
    },
}

impl std::fmt::Display for TrustError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotEnrolled { fingerprint } => {
                write!(f, "signer not enrolled: {fingerprint}")
            }
            Self::Revoked { fingerprint } => write!(f, "signer revoked: {fingerprint}"),
            Self::Expired { fingerprint } => write!(f, "signer expired: {fingerprint}"),
            Self::NamespaceDenied {
                fingerprint,
                namespace,
            } => write!(f, "signer {fingerprint} denied for namespace {namespace}"),
            Self::CapabilityDenied {
                fingerprint,
                capability,
            } => write!(
                f,
                "signer {fingerprint} denied for capability {capability}"
            ),
            Self::AlreadyEnrolled { fingerprint } => {
                write!(f, "signer already enrolled: {fingerprint}")
            }
        }
    }
}

impl std::error::Error for TrustError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustDecision {
    pub fingerprint: String,
    pub policy_revision: u64,
    pub check: TrustCheck,
}

/// In-memory authorized release-signer set (never grants trust from key bytes alone).
#[derive(Clone, Debug, Default)]
pub struct SignerTrustPolicy {
    revision: u64,
    by_fingerprint: BTreeMap<String, SignerRecord>,
}

impl SignerTrustPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn len(&self) -> usize {
        self.by_fingerprint.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_fingerprint.is_empty()
    }

    fn bump(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    /// Enroll an Ed25519 verifying key. Fingerprint is derived; never trusted from Capsule alone.
    pub fn enroll(&mut self, enrollment: SignerEnrollment) -> Result<String, TrustError> {
        let fingerprint = ed25519_fingerprint(&enrollment.public_key.0);
        if self.by_fingerprint.contains_key(&fingerprint) {
            return Err(TrustError::AlreadyEnrolled { fingerprint });
        }
        self.by_fingerprint.insert(
            fingerprint.clone(),
            SignerRecord {
                enrollment,
                fingerprint: fingerprint.clone(),
                revoked: false,
            },
        );
        self.bump();
        Ok(fingerprint)
    }

    /// Revoke by fingerprint. Prevents new declarations/reintroduction; does not rewrite Capsules.
    pub fn revoke(&mut self, fingerprint: &str) -> Result<(), TrustError> {
        let rec = self
            .by_fingerprint
            .get_mut(fingerprint)
            .ok_or_else(|| TrustError::NotEnrolled {
                fingerprint: fingerprint.to_string(),
            })?;
        rec.revoked = true;
        self.bump();
        Ok(())
    }

    pub fn is_revoked(&self, fingerprint: &str) -> bool {
        self.by_fingerprint
            .get(fingerprint)
            .map(|r| r.revoked)
            .unwrap_or(false)
    }

    /// Linearizable trust check for publication or declaration.
    pub fn check(
        &self,
        public_key: &VerifyingKeyBytes,
        namespace: &str,
        now_ms: u64,
        check: TrustCheck,
        required_capability: Option<&str>,
    ) -> Result<TrustDecision, TrustError> {
        let fingerprint = ed25519_fingerprint(&public_key.0);
        let rec = self
            .by_fingerprint
            .get(&fingerprint)
            .ok_or_else(|| TrustError::NotEnrolled {
                fingerprint: fingerprint.clone(),
            })?;
        if rec.revoked {
            return Err(TrustError::Revoked { fingerprint });
        }
        if let Some(exp) = rec.enrollment.expires_at_ms {
            if now_ms >= exp {
                return Err(TrustError::Expired { fingerprint });
            }
        }
        if !namespace_allowed(&rec.enrollment.namespaces, namespace) {
            return Err(TrustError::NamespaceDenied {
                fingerprint,
                namespace: namespace.to_string(),
            });
        }
        if let Some(cap) = required_capability {
            if !capability_allowed(&rec.enrollment.capabilities, cap) {
                return Err(TrustError::CapabilityDenied {
                    fingerprint,
                    capability: cap.to_string(),
                });
            }
        }
        let _ = check; // both publication and declaration use the same enrolled set
        Ok(TrustDecision {
            fingerprint,
            policy_revision: self.revision,
            check,
        })
    }
}

fn namespace_allowed(scopes: &BTreeSet<String>, namespace: &str) -> bool {
    scopes.iter().any(|s| s == "*" || s == namespace)
}

fn capability_allowed(caps: &BTreeSet<String>, required: &str) -> bool {
    if caps.is_empty() {
        return true;
    }
    caps.iter().any(|c| c == "*" || c == required)
}
