//! S02 exit evidence: release signer enrollment — revocation and scope matrix.
//!
//! Authority: docs/v1/DELIVERY.md S02, docs/v1/SECURITY.md §4.

use std::collections::BTreeSet;

use gump_crypto::{
    ed25519_fingerprint, generate_signing_key, verifying_key, SignerEnrollment, SignerTrustPolicy,
    SigningKeyBytes, TrustCheck, TrustError, VerifyingKeyBytes,
};
use rand_core::{TryCryptoRng, TryRng};

/// Minimal deterministic RNG for tests (not for production keys).
struct SeedRng {
    state: u64,
}

impl SeedRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }
}

impl TryRng for SeedRng {
    type Error = core::convert::Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
        Ok((self.state >> 32) as u32)
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        let a = self.try_next_u32()? as u64;
        let b = self.try_next_u32()? as u64;
        Ok((a << 32) | b)
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Self::Error> {
        for chunk in dest.chunks_mut(4) {
            let n = self.try_next_u32()?.to_le_bytes();
            for (i, b) in chunk.iter_mut().enumerate() {
                *b = n[i];
            }
        }
        Ok(())
    }
}

impl TryCryptoRng for SeedRng {}

fn keypair(seed: u64) -> (SigningKeyBytes, VerifyingKeyBytes) {
    let mut rng = SeedRng::new(seed);
    let sk = generate_signing_key(&mut rng);
    let vk = verifying_key(&sk);
    (sk, vk)
}

fn enroll_key(
    policy: &mut SignerTrustPolicy,
    seed: u64,
    namespaces: &[&str],
    expires_at_ms: Option<u64>,
    capabilities: &[&str],
) -> (VerifyingKeyBytes, String) {
    let (_sk, vk) = keypair(seed);
    let fp = policy
        .enroll(SignerEnrollment {
            public_key: vk,
            namespaces: namespaces.iter().map(|s| (*s).to_string()).collect(),
            expires_at_ms,
            capabilities: capabilities.iter().map(|s| (*s).to_string()).collect(),
        })
        .unwrap();
    assert_eq!(fp, ed25519_fingerprint(&vk.0));
    (vk, fp)
}

#[test]
fn embedded_key_alone_grants_no_trust() {
    let policy = SignerTrustPolicy::new();
    let (_sk, vk) = keypair(1);
    let err = policy
        .check(&vk, "prod", 0, TrustCheck::Publication, None)
        .unwrap_err();
    assert!(matches!(err, TrustError::NotEnrolled { .. }));
}

#[test]
fn enrollment_allows_publication_and_declaration_checks() {
    let mut policy = SignerTrustPolicy::new();
    let (vk, fp) = enroll_key(&mut policy, 2, &["prod"], None, &[]);
    let pub_ok = policy
        .check(&vk, "prod", 0, TrustCheck::Publication, None)
        .unwrap();
    assert_eq!(pub_ok.fingerprint, fp);
    let dec_ok = policy
        .check(&vk, "prod", 0, TrustCheck::Declaration, None)
        .unwrap();
    assert_eq!(dec_ok.fingerprint, fp);
    assert_eq!(dec_ok.policy_revision, policy.revision());
}

#[test]
fn namespace_scope_matrix() {
    let mut policy = SignerTrustPolicy::new();
    let (vk, _) = enroll_key(&mut policy, 3, &["prod", "staging"], None, &[]);

    assert!(policy
        .check(&vk, "prod", 0, TrustCheck::Publication, None)
        .is_ok());
    assert!(policy
        .check(&vk, "staging", 0, TrustCheck::Declaration, None)
        .is_ok());
    let denied = policy
        .check(&vk, "dev", 0, TrustCheck::Publication, None)
        .unwrap_err();
    assert!(matches!(
        denied,
        TrustError::NamespaceDenied { namespace, .. } if namespace == "dev"
    ));
}

#[test]
fn wildcard_namespace_allows_all() {
    let mut policy = SignerTrustPolicy::new();
    let (vk, _) = enroll_key(&mut policy, 4, &["*"], None, &[]);
    assert!(policy
        .check(&vk, "any-ns", 1, TrustCheck::Publication, None)
        .is_ok());
}

#[test]
fn revocation_blocks_new_checks_without_rewriting_enrollment() {
    let mut policy = SignerTrustPolicy::new();
    let (vk, fp) = enroll_key(&mut policy, 5, &["prod"], None, &[]);
    assert!(policy
        .check(&vk, "prod", 0, TrustCheck::Declaration, None)
        .is_ok());
    policy.revoke(&fp).unwrap();
    assert!(policy.is_revoked(&fp));
    let err = policy
        .check(&vk, "prod", 0, TrustCheck::Declaration, None)
        .unwrap_err();
    assert!(matches!(err, TrustError::Revoked { .. }));
    assert_eq!(policy.len(), 1);
}

#[test]
fn expiry_denies_after_deadline() {
    let mut policy = SignerTrustPolicy::new();
    let (vk, _) = enroll_key(&mut policy, 6, &["prod"], Some(1_000), &[]);
    assert!(policy
        .check(&vk, "prod", 999, TrustCheck::Publication, None)
        .is_ok());
    let err = policy
        .check(&vk, "prod", 1_000, TrustCheck::Publication, None)
        .unwrap_err();
    assert!(matches!(err, TrustError::Expired { .. }));
}

#[test]
fn capability_constraint_matrix() {
    let mut policy = SignerTrustPolicy::new();
    let (vk, _) = enroll_key(&mut policy, 7, &["prod"], None, &["deploy", "reintroduce"]);
    assert!(policy
        .check(&vk, "prod", 0, TrustCheck::Declaration, Some("deploy"))
        .is_ok());
    let err = policy
        .check(&vk, "prod", 0, TrustCheck::Declaration, Some("purge"))
        .unwrap_err();
    assert!(matches!(
        err,
        TrustError::CapabilityDenied { capability, .. } if capability == "purge"
    ));
}

#[test]
fn empty_capabilities_are_unrestricted_within_namespace() {
    let mut policy = SignerTrustPolicy::new();
    let (vk, _) = enroll_key(&mut policy, 8, &["prod"], None, &[]);
    assert!(policy
        .check(&vk, "prod", 0, TrustCheck::Publication, Some("anything"))
        .is_ok());
}

#[test]
fn duplicate_enrollment_rejected() {
    let mut policy = SignerTrustPolicy::new();
    let (_sk, vk) = keypair(9);
    let enrollment = SignerEnrollment {
        public_key: vk,
        namespaces: BTreeSet::from(["prod".into()]),
        expires_at_ms: None,
        capabilities: BTreeSet::new(),
    };
    policy.enroll(enrollment.clone()).unwrap();
    assert!(matches!(
        policy.enroll(enrollment),
        Err(TrustError::AlreadyEnrolled { .. })
    ));
}
