//! D04 exit evidence: declaration accept with concurrent generation CAS.
//!
//! Authority: docs/v1/DELIVERY.md D04, PROTOCOL.md §13, FORMATS.md §12, INV-015.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::thread;

use gump_connectors::{DeclarationDraft, DeclarationError, DeclarationLedger, sign_declaration};
use gump_crypto::{
    SignerEnrollment, SignerTrustPolicy, SigningKeyBytes, VerifyingKeyBytes, generate_signing_key,
    verifying_key,
};
use gump_types::{Action, CapsuleId, PolicyEngine, PrincipalId, Role, WorkloadId};
use rand_core::{TryCryptoRng, TryRng};

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

fn v7(seed: u8) -> [u8; 16] {
    let mut b = [seed; 16];
    b[6] = (b[6] & 0x0f) | 0x70;
    b[8] = (b[8] & 0x3f) | 0x80;
    b
}

fn wid() -> WorkloadId {
    WorkloadId::from_bytes(v7(0x71)).unwrap()
}

fn draft(digest: [u8; 32], expected_generation: u64, op: u64) -> DeclarationDraft {
    DeclarationDraft {
        namespace: "prod".into(),
        app_name: "accounts".into(),
        workload_id: Some(wid()),
        expected_generation,
        capsule_id: CapsuleId::from_bytes(v7(0x61)).unwrap(),
        capsule_digest: digest,
        lifecycle: "finite".into(),
        units: 1,
        operation_id: op,
        deployer_principal: "oidc:deployer".into(),
    }
}

fn setup(
    seed: u64,
) -> (
    PolicyEngine,
    SignerTrustPolicy,
    PrincipalId,
    SigningKeyBytes,
    VerifyingKeyBytes,
) {
    let mut policy = PolicyEngine::new();
    let principal = PrincipalId::new("oidc:deployer").unwrap();
    policy.bind_role(principal.clone(), Role::Deployer);

    let mut rng = SeedRng::new(seed);
    let sk = generate_signing_key(&mut rng);
    let vk = verifying_key(&sk);
    let mut trust = SignerTrustPolicy::new();
    trust
        .enroll(SignerEnrollment {
            public_key: vk,
            namespaces: BTreeSet::from(["prod".into()]),
            expires_at_ms: None,
            capabilities: BTreeSet::from(["deploy".into()]),
        })
        .unwrap();
    (policy, trust, principal, sk, vk)
}

fn accept(
    ledger: &mut DeclarationLedger,
    policy: &mut PolicyEngine,
    trust: &SignerTrustPolicy,
    principal: &PrincipalId,
    sk: &SigningKeyBytes,
    vk: &VerifyingKeyBytes,
    d: DeclarationDraft,
) -> Result<gump_connectors::AcceptResult, DeclarationError> {
    let next_gen = d.expected_generation + 1;
    let workload_id = d.workload_id.unwrap_or_else(wid);
    let sig = sign_declaration(sk, &d, workload_id, next_gen).unwrap();
    ledger.accept_declaration(policy, trust, principal, vk, &sig, d, "prod", 0)
}

#[test]
fn first_accept_creates_generation_one() {
    let (mut policy, trust, principal, sk, vk) = setup(1);
    let mut ledger = DeclarationLedger::new();
    let result = accept(
        &mut ledger,
        &mut policy,
        &trust,
        &principal,
        &sk,
        &vk,
        draft([1u8; 32], 0, 10),
    )
    .unwrap();
    assert!(result.created);
    assert_eq!(result.declaration.generation, 1);
    assert_eq!(result.declaration.workload_id, wid());
    assert!(
        result
            .declaration
            .authorization_decision_id
            .starts_with("pd-")
    );
}

#[test]
fn policy_deny_blocks_accept() {
    let (_, trust, _, sk, vk) = setup(3);
    let mut policy = PolicyEngine::new();
    let principal = PrincipalId::new("oidc:nobody").unwrap();
    let mut ledger = DeclarationLedger::new();
    let err = accept(
        &mut ledger,
        &mut policy,
        &trust,
        &principal,
        &sk,
        &vk,
        draft([1u8; 32], 0, 1),
    )
    .unwrap_err();
    assert!(matches!(err, DeclarationError::PolicyDenied { .. }));
}

#[test]
fn bad_signature_rejected() {
    let (mut policy, trust, principal, sk, vk) = setup(4);
    let mut ledger = DeclarationLedger::new();
    let d = draft([1u8; 32], 0, 1);
    let mut bad = sign_declaration(&sk, &d, wid(), 1).unwrap();
    bad[0] ^= 0xff;
    let err = ledger
        .accept_declaration(&mut policy, &trust, &principal, &vk, &bad, d, "prod", 0)
        .unwrap_err();
    assert_eq!(err, DeclarationError::Signature);
}

#[test]
fn stale_expected_generation_conflicts() {
    let (mut policy, trust, principal, sk, vk) = setup(5);
    let mut ledger = DeclarationLedger::new();
    accept(
        &mut ledger,
        &mut policy,
        &trust,
        &principal,
        &sk,
        &vk,
        draft([1u8; 32], 0, 1),
    )
    .unwrap();
    let err = accept(
        &mut ledger,
        &mut policy,
        &trust,
        &principal,
        &sk,
        &vk,
        draft([2u8; 32], 0, 2),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        DeclarationError::GenerationConflict {
            current: 1,
            expected: 0
        }
    ));
}

#[test]
fn concurrent_accepts_only_one_next_generation() {
    let (mut policy, trust, principal, sk, vk) = setup(2);
    let mut ledger = DeclarationLedger::new();
    accept(
        &mut ledger,
        &mut policy,
        &trust,
        &principal,
        &sk,
        &vk,
        draft([9u8; 32], 0, 1),
    )
    .unwrap();
    assert_eq!(ledger.get("prod", "accounts").unwrap().generation, 1);

    let ledger = Arc::new(Mutex::new(ledger));
    let trust = Arc::new(trust);
    let principal = Arc::new(principal);
    let vk = Arc::new(vk);
    let sk = Arc::new(sk);

    let mut handles = Vec::new();
    for i in 0..8u64 {
        let ledger = Arc::clone(&ledger);
        let trust = Arc::clone(&trust);
        let principal = Arc::clone(&principal);
        let vk = Arc::clone(&vk);
        let sk = Arc::clone(&sk);
        handles.push(thread::spawn(move || {
            let mut digest = [2u8; 32];
            digest[0] = i as u8;
            let d = draft(digest, 1, 100 + i);
            let next_gen = 2u64;
            let sig = sign_declaration(&sk, &d, wid(), next_gen).unwrap();
            let mut policy = PolicyEngine::new();
            policy.bind_role((*principal).clone(), Role::Deployer);
            let mut guard = ledger.lock().unwrap();
            guard.accept_declaration(&mut policy, &trust, &principal, &vk, &sig, d, "prod", 0)
        }));
    }

    let mut wins = 0usize;
    let mut conflicts = 0usize;
    let mut divergent = 0usize;
    for h in handles {
        match h.join().unwrap() {
            Ok(r) => {
                assert_eq!(r.declaration.generation, 2);
                wins += 1;
            }
            Err(DeclarationError::GenerationConflict { .. }) => conflicts += 1,
            Err(DeclarationError::DivergentContent { generation }) => {
                assert_eq!(generation, 2);
                divergent += 1;
            }
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }
    assert_eq!(wins, 1, "exactly one accept must create generation 2");
    assert_eq!(wins + conflicts + divergent, 8, "all racers accounted for");
    assert_eq!(
        ledger
            .lock()
            .unwrap()
            .get("prod", "accounts")
            .unwrap()
            .generation,
        2
    );
}

#[test]
fn sequential_update_advances_generation() {
    let (mut policy, trust, principal, sk, vk) = setup(6);
    let mut ledger = DeclarationLedger::new();
    accept(
        &mut ledger,
        &mut policy,
        &trust,
        &principal,
        &sk,
        &vk,
        draft([1u8; 32], 0, 1),
    )
    .unwrap();
    let r2 = accept(
        &mut ledger,
        &mut policy,
        &trust,
        &principal,
        &sk,
        &vk,
        draft([2u8; 32], 1, 2),
    )
    .unwrap();
    assert!(!r2.created);
    assert_eq!(r2.declaration.generation, 2);
}

#[test]
fn reader_role_cannot_deploy() {
    let (_, trust, _, sk, vk) = setup(7);
    let mut policy = PolicyEngine::new();
    let principal = PrincipalId::new("oidc:reader").unwrap();
    policy.bind_role(principal.clone(), Role::Reader);
    let mut ledger = DeclarationLedger::new();
    let err = accept(
        &mut ledger,
        &mut policy,
        &trust,
        &principal,
        &sk,
        &vk,
        draft([1u8; 32], 0, 1),
    )
    .unwrap_err();
    assert!(matches!(err, DeclarationError::PolicyDenied { .. }));
    let _ = Action::WorkloadDeploy;
}
