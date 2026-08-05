//! External HSM/KMS unseal provider trait (S04 / SECURITY.md §6 / D005).
//!
//! Providers return the same logical unwrap capability as software-derived
//! cluster keys. Gump stores only provider type and non-secret key ID in live
//! memory and Capsule envelopes — never provider credentials or raw unseal
//! material.

use core::fmt;

use rand_core::CryptoRng;

use crate::aead::DEK_LEN;
use crate::error::CryptoError;
use crate::hpke_wrap::{
    generate_x25519_keypair, open_dek, seal_dek, ClusterX25519Public, ClusterX25519Secret,
    SealedDek,
};
use crate::unseal::{derive_cluster_unseal_keypair, RecoverySecret};

/// Non-secret handle recorded in memory / Capsule envelopes (`cluster_key_id`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnsealProviderDescriptor {
    /// Provider class (`software`, `fake-hsm`, cloud KMS type, …).
    pub provider_type: String,
    /// Non-secret key identifier; grants no authority by itself.
    pub key_id: String,
}

impl UnsealProviderDescriptor {
    pub fn new(provider_type: impl Into<String>, key_id: impl Into<String>) -> Self {
        Self {
            provider_type: provider_type.into(),
            key_id: key_id.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnsealProviderError {
    Crypto(CryptoError),
    KeyMismatch {
        expected: String,
        requested: String,
    },
    Unauthorized,
    Unavailable {
        reason: String,
    },
}

impl fmt::Display for UnsealProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Crypto(e) => write!(f, "{e}"),
            Self::KeyMismatch {
                expected,
                requested,
            } => write!(
                f,
                "unseal key_id mismatch: provider={expected} requested={requested}"
            ),
            Self::Unauthorized => write!(f, "unseal provider unauthorized"),
            Self::Unavailable { reason } => write!(f, "unseal provider unavailable: {reason}"),
        }
    }
}

impl std::error::Error for UnsealProviderError {}

impl From<CryptoError> for UnsealProviderError {
    fn from(e: CryptoError) -> Self {
        Self::Crypto(e)
    }
}

/// Cluster unseal authority backed by software ceremony or external HSM/KMS.
pub trait UnsealProvider {
    fn descriptor(&self) -> &UnsealProviderDescriptor;

    /// Public X25519 key used by clients to seal DEKs (safe to distribute).
    fn cluster_public(&self) -> ClusterX25519Public;

    /// Unwrap a sealed DEK after the caller has selected this live authority.
    ///
    /// `requested_key_id` is the Capsule/`cluster_key_id` handle; it must match
    /// this provider's descriptor. Providers must not expose raw private keys.
    fn unwrap_dek(
        &self,
        requested_key_id: &str,
        sealed: &SealedDek,
        info: &[u8],
        aad: &[u8],
    ) -> Result<[u8; DEK_LEN], UnsealProviderError>;
}

/// Software unseal: recovery secret → HKDF → X25519 (S03), exposed via the trait.
pub struct SoftwareUnsealProvider {
    descriptor: UnsealProviderDescriptor,
    secret: ClusterX25519Secret,
    public: ClusterX25519Public,
}

impl SoftwareUnsealProvider {
    pub fn from_recovery_secret(
        secret: &RecoverySecret,
        cluster_id: &[u8; 16],
        key_id: impl Into<String>,
    ) -> Result<Self, CryptoError> {
        let (sk, pk) = derive_cluster_unseal_keypair(secret, cluster_id)?;
        Ok(Self {
            descriptor: UnsealProviderDescriptor::new("software", key_id),
            secret: sk,
            public: pk,
        })
    }
}

impl UnsealProvider for SoftwareUnsealProvider {
    fn descriptor(&self) -> &UnsealProviderDescriptor {
        &self.descriptor
    }

    fn cluster_public(&self) -> ClusterX25519Public {
        self.public
    }

    fn unwrap_dek(
        &self,
        requested_key_id: &str,
        sealed: &SealedDek,
        info: &[u8],
        aad: &[u8],
    ) -> Result<[u8; DEK_LEN], UnsealProviderError> {
        ensure_key_id(&self.descriptor, requested_key_id)?;
        Ok(open_dek(
            &self.secret,
            &sealed.encapsulated_key,
            info,
            aad,
            &sealed.wrapped_dek,
        )?)
    }
}

impl fmt::Debug for SoftwareUnsealProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SoftwareUnsealProvider")
            .field("descriptor", &self.descriptor)
            .field("public", &self.public)
            .finish_non_exhaustive()
    }
}

/// In-process fake HSM/KMS for conformance (credentials stay inside the fake).
///
/// Holds the private key only in a zeroizing field and never returns it through
/// the trait. Simulates authorization and availability failures.
pub struct FakeHsmUnsealProvider {
    descriptor: UnsealProviderDescriptor,
    secret: ClusterX25519Secret,
    public: ClusterX25519Public,
    authorized: bool,
    available: bool,
}

impl FakeHsmUnsealProvider {
    pub const PROVIDER_TYPE: &'static str = "fake-hsm";

    pub fn generate<R: CryptoRng>(rng: &mut R, key_id: impl Into<String>) -> Self {
        let (sk, pk) = generate_x25519_keypair(rng);
        Self {
            descriptor: UnsealProviderDescriptor::new(Self::PROVIDER_TYPE, key_id),
            secret: sk,
            public: pk,
            authorized: true,
            available: true,
        }
    }

    pub fn set_authorized(&mut self, authorized: bool) {
        self.authorized = authorized;
    }

    pub fn set_available(&mut self, available: bool) {
        self.available = available;
    }
}

impl UnsealProvider for FakeHsmUnsealProvider {
    fn descriptor(&self) -> &UnsealProviderDescriptor {
        &self.descriptor
    }

    fn cluster_public(&self) -> ClusterX25519Public {
        self.public
    }

    fn unwrap_dek(
        &self,
        requested_key_id: &str,
        sealed: &SealedDek,
        info: &[u8],
        aad: &[u8],
    ) -> Result<[u8; DEK_LEN], UnsealProviderError> {
        if !self.available {
            return Err(UnsealProviderError::Unavailable {
                reason: "fake HSM offline".into(),
            });
        }
        if !self.authorized {
            return Err(UnsealProviderError::Unauthorized);
        }
        ensure_key_id(&self.descriptor, requested_key_id)?;
        Ok(open_dek(
            &self.secret,
            &sealed.encapsulated_key,
            info,
            aad,
            &sealed.wrapped_dek,
        )?)
    }
}

impl fmt::Debug for FakeHsmUnsealProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FakeHsmUnsealProvider")
            .field("descriptor", &self.descriptor)
            .field("public", &self.public)
            .field("authorized", &self.authorized)
            .field("available", &self.available)
            .finish_non_exhaustive()
    }
}

fn ensure_key_id(
    descriptor: &UnsealProviderDescriptor,
    requested_key_id: &str,
) -> Result<(), UnsealProviderError> {
    if descriptor.key_id != requested_key_id {
        return Err(UnsealProviderError::KeyMismatch {
            expected: descriptor.key_id.clone(),
            requested: requested_key_id.into(),
        });
    }
    Ok(())
}

/// Seal to a provider's public key then unwrap via the trait (same HPKE suite).
pub fn seal_and_unwrap_via_provider<R: CryptoRng, P: UnsealProvider>(
    rng: &mut R,
    provider: &P,
    info: &[u8],
    aad: &[u8],
    dek: &[u8; DEK_LEN],
) -> Result<[u8; DEK_LEN], UnsealProviderError> {
    let sealed = seal_dek(rng, &provider.cluster_public(), info, aad, dek)?;
    provider.unwrap_dek(&provider.descriptor().key_id, &sealed, info, aad)
}
