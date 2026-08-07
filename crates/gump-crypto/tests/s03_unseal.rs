//! S03 exit evidence: software unseal vectors, zeroization, failure paths.
//!
//! Authority: docs/v1/SECURITY.md §6, DECISIONS D005, DELIVERY S03.

use gump_crypto::{
    CLUSTER_UNSEAL_INFO, CryptoErrorKind, DEFAULT_SHARE_COUNT, DEFAULT_THRESHOLD, OperatorShare,
    RECOVERY_SECRET_LEN, RecoverySecret, combine_recovery_shares, derive_cluster_unseal_keypair,
    generate_recovery_secret, open_dek, seal_dek, split_recovery_secret,
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
    let mut b = [0x11u8; 16];
    b[6] = (b[6] & 0x0f) | 0x70;
    b[8] = (b[8] & 0x3f) | 0x80;
    b
}

#[test]
fn one_of_one_roundtrip_and_unseal_derive() {
    let mut rng = SeedRng::new(1);
    let secret = generate_recovery_secret(&mut rng);
    let shares = split_recovery_secret(&secret, 1, 1, &mut rng).unwrap();
    assert_eq!(shares.len(), 1);
    let recovered = combine_recovery_shares(&shares, 1).unwrap();
    assert_eq!(recovered.as_bytes(), secret.as_bytes());

    let (sk, pk) = derive_cluster_unseal_keypair(&recovered, &cluster_id()).unwrap();
    let mut rng = SeedRng::new(2);
    let dek = [0x5au8; 32];
    let sealed = seal_dek(&mut rng, &pk, CLUSTER_UNSEAL_INFO, b"aad", &dek).unwrap();
    let opened = open_dek(
        &sk,
        &sealed.encapsulated_key,
        CLUSTER_UNSEAL_INFO,
        b"aad",
        &sealed.wrapped_dek,
    )
    .unwrap();
    assert_eq!(opened.expose(), &dek);
}

#[test]
fn three_of_five_any_threshold_subset_recovers() {
    let mut rng = SeedRng::new(3);
    let secret = generate_recovery_secret(&mut rng);
    let shares =
        split_recovery_secret(&secret, DEFAULT_SHARE_COUNT, DEFAULT_THRESHOLD, &mut rng).unwrap();
    assert_eq!(shares.len(), DEFAULT_SHARE_COUNT as usize);

    // Every combination of 3 distinct shares must recover.
    for i in 0..5 {
        for j in (i + 1)..5 {
            for k in (j + 1)..5 {
                let subset = [shares[i].clone(), shares[j].clone(), shares[k].clone()];
                let recovered = combine_recovery_shares(&subset, DEFAULT_THRESHOLD).unwrap();
                assert_eq!(recovered.as_bytes(), secret.as_bytes());
            }
        }
    }
}

#[test]
fn fewer_than_threshold_shares_fail() {
    let mut rng = SeedRng::new(4);
    let secret = generate_recovery_secret(&mut rng);
    let shares = split_recovery_secret(&secret, 5, 3, &mut rng).unwrap();
    let err = combine_recovery_shares(&shares[..2], 3).unwrap_err();
    assert_eq!(err.kind(), CryptoErrorKind::Share);
}

#[test]
fn invalid_params_rejected() {
    let mut rng = SeedRng::new(5);
    let secret = generate_recovery_secret(&mut rng);
    assert_eq!(
        split_recovery_secret(&secret, 2, 3, &mut rng)
            .unwrap_err()
            .kind(),
        CryptoErrorKind::Share
    );
    assert_eq!(
        split_recovery_secret(&secret, 0, 1, &mut rng)
            .unwrap_err()
            .kind(),
        CryptoErrorKind::Share
    );
    assert_eq!(
        split_recovery_secret(&secret, 3, 0, &mut rng)
            .unwrap_err()
            .kind(),
        CryptoErrorKind::Share
    );
}

#[test]
fn derive_unseal_kat_vector() {
    // Fixed IKM + cluster_id → stable public key (regression vector).
    let secret = RecoverySecret::from_bytes([0x42u8; RECOVERY_SECRET_LEN]);
    let (_sk, pk) = derive_cluster_unseal_keypair(&secret, &cluster_id()).unwrap();
    let expected: [u8; 32] = [
        0xf1, 0x96, 0xe6, 0x04, 0x61, 0xa2, 0xd1, 0x81, 0x92, 0x21, 0x33, 0xb6, 0xf5, 0xb5, 0x32,
        0xd6, 0x8e, 0xae, 0x21, 0x16, 0x0f, 0x69, 0x4b, 0x74, 0x6f, 0xaa, 0xd5, 0x87, 0xd1, 0xea,
        0xca, 0x0d,
    ];
    assert_eq!(pk.0, expected);
}

#[test]
fn secrets_redacted_in_debug() {
    let secret = RecoverySecret::from_bytes([0x99; 32]);
    let dbg = format!("{secret:?}");
    assert!(dbg.contains("REDACTED"));
    assert!(!dbg.contains("99"));
}

#[test]
fn empty_and_corrupt_share_fail() {
    assert_eq!(
        OperatorShare::from_bytes(vec![]).unwrap_err().kind(),
        CryptoErrorKind::Length
    );
    assert_eq!(
        OperatorShare::from_bytes(vec![1]).unwrap_err().kind(),
        CryptoErrorKind::Share
    );
}

#[test]
fn different_cluster_id_different_unseal_key() {
    let secret = RecoverySecret::from_bytes([0x07; 32]);
    let mut other = cluster_id();
    other[0] ^= 0xff;
    let (_, pk_a) = derive_cluster_unseal_keypair(&secret, &cluster_id()).unwrap();
    let (_, pk_b) = derive_cluster_unseal_keypair(&secret, &other).unwrap();
    assert_ne!(pk_a.0, pk_b.0);
}
