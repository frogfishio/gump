//! S04 exit evidence: fake HSM/KMS unseal provider conformance suite.
//!
//! Authority: DELIVERY S04, SECURITY.md §6, DECISIONS D004–D005.

use gump_crypto::{
    seal_and_unwrap_via_provider, seal_dek, FakeHsmUnsealProvider, RecoverySecret,
    SoftwareUnsealProvider, UnsealProvider, UnsealProviderError, CLUSTER_UNSEAL_INFO,
    HPKE_SUITE_ID,
};
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

fn cluster_id() -> [u8; 16] {
    let mut b = [0x22u8; 16];
    b[6] = (b[6] & 0x0f) | 0x70;
    b[8] = (b[8] & 0x3f) | 0x80;
    b
}

#[test]
fn fake_hsm_seal_unwrap_roundtrip() {
    let mut rng = SeedRng::new(1);
    let provider = FakeHsmUnsealProvider::generate(&mut rng, "hsm-key-alpha");
    let dek = [0x11u8; 32];
    let opened = seal_and_unwrap_via_provider(
        &mut rng,
        &provider,
        CLUSTER_UNSEAL_INFO,
        b"aad-s04",
        &dek,
    )
    .unwrap();
    assert_eq!(opened, dek);
}

#[test]
fn descriptor_stores_only_type_and_key_id() {
    let mut rng = SeedRng::new(2);
    let provider = FakeHsmUnsealProvider::generate(&mut rng, "kms/projects/p/keys/k");
    let d = provider.descriptor();
    assert_eq!(d.provider_type, FakeHsmUnsealProvider::PROVIDER_TYPE);
    assert_eq!(d.key_id, "kms/projects/p/keys/k");
    let dbg = format!("{provider:?}");
    assert!(dbg.contains("fake-hsm"));
    assert!(dbg.contains("kms/projects/p/keys/k"));
    assert!(!dbg.contains("secret"));
    assert!(dbg.contains("..")); // finish_non_exhaustive
}

#[test]
fn wrong_key_id_rejected() {
    let mut rng = SeedRng::new(3);
    let provider = FakeHsmUnsealProvider::generate(&mut rng, "correct-id");
    let dek = [0x22u8; 32];
    let sealed = seal_dek(
        &mut rng,
        &provider.cluster_public(),
        CLUSTER_UNSEAL_INFO,
        b"aad",
        &dek,
    )
    .unwrap();
    let err = provider
        .unwrap_dek("other-id", &sealed, CLUSTER_UNSEAL_INFO, b"aad")
        .unwrap_err();
    assert!(matches!(
        err,
        UnsealProviderError::KeyMismatch {
            expected,
            requested
        } if expected == "correct-id" && requested == "other-id"
    ));
}

#[test]
fn unauthorized_and_unavailable_fail_closed() {
    let mut rng = SeedRng::new(4);
    let mut provider = FakeHsmUnsealProvider::generate(&mut rng, "k1");
    let dek = [0x33u8; 32];
    let sealed = seal_dek(
        &mut rng,
        &provider.cluster_public(),
        CLUSTER_UNSEAL_INFO,
        b"aad",
        &dek,
    )
    .unwrap();

    provider.set_authorized(false);
    assert_eq!(
        provider
            .unwrap_dek("k1", &sealed, CLUSTER_UNSEAL_INFO, b"aad")
            .unwrap_err(),
        UnsealProviderError::Unauthorized
    );

    provider.set_authorized(true);
    provider.set_available(false);
    assert!(matches!(
        provider
            .unwrap_dek("k1", &sealed, CLUSTER_UNSEAL_INFO, b"aad")
            .unwrap_err(),
        UnsealProviderError::Unavailable { .. }
    ));
}

#[test]
fn software_provider_same_trait_contract() {
    let mut rng = SeedRng::new(5);
    let secret = RecoverySecret::from_bytes([0x7eu8; 32]);
    let provider =
        SoftwareUnsealProvider::from_recovery_secret(&secret, &cluster_id(), "soft-1").unwrap();
    assert_eq!(provider.descriptor().provider_type, "software");
    let dek = [0x44u8; 32];
    let opened = seal_and_unwrap_via_provider(
        &mut rng,
        &provider,
        CLUSTER_UNSEAL_INFO,
        b"aad",
        &dek,
    )
    .unwrap();
    assert_eq!(opened, dek);
}

#[test]
fn provider_does_not_change_capsule_cipher_suite() {
    // D004: HSM/KMS does not change Capsule cipher suite or wire representation.
    assert_eq!(HPKE_SUITE_ID, "HPKE-X25519-HKDFSHA256-CHACHA20POLY1305");
    let mut rng = SeedRng::new(6);
    let fake = FakeHsmUnsealProvider::generate(&mut rng, "suite");
    let soft = SoftwareUnsealProvider::from_recovery_secret(
        &RecoverySecret::from_bytes([0x01; 32]),
        &cluster_id(),
        "suite-soft",
    )
    .unwrap();
    // Both expose the same public-key type and use seal_dek/open_dek under the trait.
    let _ = (fake.cluster_public().0, soft.cluster_public().0);
    let dek = [0x55u8; 32];
    assert_eq!(
        seal_and_unwrap_via_provider(&mut rng, &fake, CLUSTER_UNSEAL_INFO, b"", &dek).unwrap(),
        dek
    );
    assert_eq!(
        seal_and_unwrap_via_provider(&mut rng, &soft, CLUSTER_UNSEAL_INFO, b"", &dek).unwrap(),
        dek
    );
}

#[test]
fn foreign_provider_cannot_unwrap_foreign_seal() {
    let mut rng = SeedRng::new(7);
    let a = FakeHsmUnsealProvider::generate(&mut rng, "a");
    let b = FakeHsmUnsealProvider::generate(&mut rng, "b");
    let dek = [0x66u8; 32];
    let sealed = seal_dek(
        &mut rng,
        &a.cluster_public(),
        CLUSTER_UNSEAL_INFO,
        b"aad",
        &dek,
    )
    .unwrap();
    // Same key_id string but different HSM key material → crypto failure.
    let b_same_id = FakeHsmUnsealProvider::generate(&mut rng, "a");
    let _ = b;
    let err = b_same_id
        .unwrap_dek("a", &sealed, CLUSTER_UNSEAL_INFO, b"aad")
        .unwrap_err();
    assert!(matches!(err, UnsealProviderError::Crypto(_)));
}
