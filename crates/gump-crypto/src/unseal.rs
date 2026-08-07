//! Software unseal / Shamir share ceremony (S03 / SECURITY.md §6 / D005).
//!
//! Recovery secret is never written by Gump. Shares are operator-held only.
//! Cluster X25519 unseal keys are derived via HKDF-SHA256 then clamped by the
//! vetted HPKE X25519 implementation.

use core::fmt;

use hkdf::Hkdf;
use hpke::kem::X25519HkdfSha256;
use hpke::{Deserializable, Kem, Serializable};
use rand_chacha::ChaCha20Rng;
use rand_chacha::rand_core::SeedableRng;
use rand_core::CryptoRng;
use sha2::Sha256;
use sharks::{Share as SharkShare, Sharks};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::{CryptoError, CryptoErrorKind};
use crate::hpke_wrap::{ClusterX25519Public, ClusterX25519Secret};

/// Length of the cluster recovery secret (SECURITY.md §6).
pub const RECOVERY_SECRET_LEN: usize = 32;

/// Production default share count (SECURITY.md §6).
pub const DEFAULT_SHARE_COUNT: u8 = 5;

/// Production default threshold (SECURITY.md §6).
pub const DEFAULT_THRESHOLD: u8 = 3;

/// HKDF info for cluster unseal X25519 derivation.
pub const CLUSTER_UNSEAL_INFO: &[u8] = b"gump.cluster-unseal-x25519/1\0";

type KemSuite = X25519HkdfSha256;

/// 32-byte recovery secret (zeroized on drop; never Debug-printed; not Clone).
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct RecoverySecret([u8; RECOVERY_SECRET_LEN]);

impl RecoverySecret {
    pub fn from_bytes(bytes: [u8; RECOVERY_SECRET_LEN]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; RECOVERY_SECRET_LEN] {
        &self.0
    }
}

impl fmt::Debug for RecoverySecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RecoverySecret([REDACTED])")
    }
}

/// One operator-held Shamir share (index + share body) — not Clone (SECURITY §8).
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct OperatorShare {
    raw: Vec<u8>,
}

impl OperatorShare {
    pub fn from_bytes(raw: Vec<u8>) -> Result<Self, CryptoError> {
        if raw.is_empty() {
            return Err(CryptoError::new(CryptoErrorKind::Length, "empty share"));
        }
        // Validate encoding early.
        let _ = SharkShare::try_from(raw.as_slice()).map_err(|e| {
            CryptoError::new(
                CryptoErrorKind::Share,
                format!("invalid share encoding: {e}"),
            )
        })?;
        Ok(Self { raw })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.raw
    }

    pub fn x_coordinate(&self) -> u8 {
        self.raw[0]
    }
}

impl fmt::Debug for OperatorShare {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "OperatorShare {{ x: {}, len: {} }}",
            self.raw.first().copied().unwrap_or(0),
            self.raw.len()
        )
    }
}

/// Generate a fresh recovery secret from the OS/CSPRNG path.
pub fn generate_recovery_secret<R: CryptoRng>(rng: &mut R) -> RecoverySecret {
    let mut bytes = [0u8; RECOVERY_SECRET_LEN];
    fill_crypto(rng, &mut bytes);
    RecoverySecret(bytes)
}

/// Split a recovery secret into `n` shares with threshold `t` (Shamir over GF(256)).
///
/// Gump must not retain shares or the secret after the operator has copied them
/// out (SECURITY.md §6 / D005).
pub fn split_recovery_secret<R: CryptoRng>(
    secret: &RecoverySecret,
    n: u8,
    t: u8,
    rng: &mut R,
) -> Result<Vec<OperatorShare>, CryptoError> {
    validate_params(n, t)?;
    let mut seed = [0u8; 32];
    fill_crypto(rng, &mut seed);
    let mut share_rng = ChaCha20Rng::from_seed(seed);
    seed.zeroize();

    let sharks = Sharks(t);
    let shares: Vec<OperatorShare> = sharks
        .dealer_rng(secret.as_bytes(), &mut share_rng)
        .take(n as usize)
        .map(|s| OperatorShare {
            raw: Vec::<u8>::from(&s),
        })
        .collect();
    if shares.len() != n as usize {
        return Err(CryptoError::new(
            CryptoErrorKind::Share,
            format!("dealer produced {} shares, expected {n}", shares.len()),
        ));
    }
    Ok(shares)
}

/// Reconstruct the recovery secret from at least `threshold` shares.
pub fn combine_recovery_shares(
    shares: &[OperatorShare],
    threshold: u8,
) -> Result<RecoverySecret, CryptoError> {
    if threshold == 0 {
        return Err(CryptoError::new(
            CryptoErrorKind::Share,
            "threshold must be >= 1",
        ));
    }
    if shares.len() < threshold as usize {
        return Err(CryptoError::new(
            CryptoErrorKind::Share,
            format!("need at least {threshold} shares, got {}", shares.len()),
        ));
    }

    let parsed: Result<Vec<SharkShare>, CryptoError> = shares
        .iter()
        .map(|s| {
            SharkShare::try_from(s.as_bytes()).map_err(|e| {
                CryptoError::new(CryptoErrorKind::Share, format!("invalid share: {e}"))
            })
        })
        .collect();
    let parsed = parsed?;

    let sharks = Sharks(threshold);
    let secret = sharks
        .recover(&parsed)
        .map_err(|e| CryptoError::new(CryptoErrorKind::Share, format!("recover failed: {e}")))?;
    if secret.len() != RECOVERY_SECRET_LEN {
        return Err(CryptoError::new(
            CryptoErrorKind::Length,
            format!(
                "recovered secret length {}, expected {RECOVERY_SECRET_LEN}",
                secret.len()
            ),
        ));
    }
    let mut bytes = [0u8; RECOVERY_SECRET_LEN];
    bytes.copy_from_slice(&secret);
    Ok(RecoverySecret(bytes))
}

/// Derive the cluster X25519 unseal keypair (SECURITY.md §6).
///
/// ```text
/// HKDF-SHA256(
///   ikm = recovery_secret,
///   salt = cluster_id,
///   info = "gump.cluster-unseal-x25519/1\0"
/// )
/// ```
pub fn derive_cluster_unseal_keypair(
    secret: &RecoverySecret,
    cluster_id: &[u8; 16],
) -> Result<(ClusterX25519Secret, ClusterX25519Public), CryptoError> {
    let hk = Hkdf::<Sha256>::new(Some(cluster_id.as_slice()), secret.as_bytes());
    let mut okm = [0u8; 32];
    hk.expand(CLUSTER_UNSEAL_INFO, &mut okm)
        .map_err(|_| CryptoError::new(CryptoErrorKind::Unseal, "HKDF-SHA256 expand failed"))?;

    let sk = <KemSuite as Kem>::PrivateKey::from_bytes(&okm).map_err(|e| {
        CryptoError::new(
            CryptoErrorKind::Unseal,
            format!("X25519 clamp/deserialize failed: {e}"),
        )
    })?;
    okm.zeroize();
    let pk = KemSuite::sk_to_pk(&sk);
    let sk_bytes = sk.to_bytes();
    let pk_bytes = pk.to_bytes();
    let mut sk_arr = [0u8; 32];
    let mut pk_arr = [0u8; 32];
    sk_arr.copy_from_slice(sk_bytes.as_slice());
    pk_arr.copy_from_slice(pk_bytes.as_slice());
    Ok((
        ClusterX25519Secret::from_bytes(sk_arr),
        ClusterX25519Public(pk_arr),
    ))
}

fn validate_params(n: u8, t: u8) -> Result<(), CryptoError> {
    if t == 0 {
        return Err(CryptoError::new(
            CryptoErrorKind::Share,
            "threshold must be >= 1",
        ));
    }
    if n == 0 {
        return Err(CryptoError::new(
            CryptoErrorKind::Share,
            "share count must be >= 1",
        ));
    }
    if t > n {
        return Err(CryptoError::new(
            CryptoErrorKind::Share,
            format!("threshold {t} exceeds share count {n}"),
        ));
    }
    Ok(())
}

fn fill_crypto<R: CryptoRng>(rng: &mut R, dest: &mut [u8]) {
    rng.fill_bytes(dest);
}
