//! HPKE DEK seal to cluster X25519 unseal key (FORMATS.md §8 / SECURITY §5).

use hpke::aead::ChaCha20Poly1305;
use hpke::kdf::HkdfSha256;
use hpke::kem::X25519HkdfSha256;
use hpke::{Deserializable, Kem, OpModeR, OpModeS, Serializable};
use rand_core::CryptoRng;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::aead::{DEK_LEN, Dek};
use crate::error::{CryptoError, CryptoErrorKind};

pub const HPKE_SUITE_ID: &str = "HPKE-X25519-HKDFSHA256-CHACHA20POLY1305";

type AeadSuite = ChaCha20Poly1305;
type Kdf = HkdfSha256;
type KemSuite = X25519HkdfSha256;

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct ClusterX25519Secret(pub [u8; 32]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterX25519Public(pub [u8; 32]);

/// Result of sealing a DEK to a cluster public key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SealedDek {
    pub encapsulated_key: [u8; 32],
    pub wrapped_dek: Vec<u8>,
}

pub fn generate_x25519_keypair<R: CryptoRng>(
    rng: &mut R,
) -> (ClusterX25519Secret, ClusterX25519Public) {
    let (sk, pk) = KemSuite::gen_keypair_with_rng(rng);
    let sk_bytes = sk.to_bytes();
    let pk_bytes = pk.to_bytes();
    let mut sk_arr = [0u8; 32];
    let mut pk_arr = [0u8; 32];
    sk_arr.copy_from_slice(sk_bytes.as_slice());
    pk_arr.copy_from_slice(pk_bytes.as_slice());
    (ClusterX25519Secret(sk_arr), ClusterX25519Public(pk_arr))
}

/// Seal the 32-byte DEK with HPKE base mode.
pub fn seal_dek<R: CryptoRng>(
    rng: &mut R,
    recipient_public: &ClusterX25519Public,
    info: &[u8],
    aad: &[u8],
    dek: &[u8; DEK_LEN],
) -> Result<SealedDek, CryptoError> {
    let pk = <KemSuite as Kem>::PublicKey::from_bytes(&recipient_public.0).map_err(|e| {
        CryptoError::new(
            CryptoErrorKind::Hpke,
            format!("invalid recipient public: {e}"),
        )
    })?;
    let (encapped, ciphertext) = hpke::single_shot_seal_with_rng::<AeadSuite, Kdf, KemSuite>(
        &OpModeS::Base,
        &pk,
        info,
        dek,
        aad,
        rng,
    )
    .map_err(|e| CryptoError::new(CryptoErrorKind::Hpke, format!("seal: {e}")))?;
    let enc_bytes = encapped.to_bytes();
    if enc_bytes.len() != 32 {
        return Err(CryptoError::new(
            CryptoErrorKind::Hpke,
            format!("unexpected enc size {}", enc_bytes.len()),
        ));
    }
    let mut encapsulated_key = [0u8; 32];
    encapsulated_key.copy_from_slice(enc_bytes.as_slice());
    Ok(SealedDek {
        encapsulated_key,
        wrapped_dek: ciphertext,
    })
}

/// Open a sealed DEK into a zeroizing container (never bare `[u8; 32]`).
pub fn open_dek(
    recipient_secret: &ClusterX25519Secret,
    encapsulated_key: &[u8; 32],
    info: &[u8],
    aad: &[u8],
    wrapped_dek: &[u8],
) -> Result<Dek, CryptoError> {
    let sk = <KemSuite as Kem>::PrivateKey::from_bytes(&recipient_secret.0).map_err(|e| {
        CryptoError::new(
            CryptoErrorKind::Hpke,
            format!("invalid recipient secret: {e}"),
        )
    })?;
    let enc = <KemSuite as Kem>::EncappedKey::from_bytes(encapsulated_key).map_err(|e| {
        CryptoError::new(CryptoErrorKind::Hpke, format!("invalid encapped key: {e}"))
    })?;
    let mut plaintext = hpke::single_shot_open::<AeadSuite, Kdf, KemSuite>(
        &OpModeR::Base,
        &sk,
        &enc,
        info,
        wrapped_dek,
        aad,
    )
    .map_err(|e| CryptoError::new(CryptoErrorKind::Hpke, format!("open: {e}")))?;
    if plaintext.len() != DEK_LEN {
        plaintext.zeroize();
        return Err(CryptoError::new(
            CryptoErrorKind::Length,
            "HPKE plaintext is not a 32-byte DEK",
        ));
    }
    let mut dek = [0u8; DEK_LEN];
    dek.copy_from_slice(&plaintext);
    plaintext.zeroize();
    Ok(Dek::new(dek))
}
