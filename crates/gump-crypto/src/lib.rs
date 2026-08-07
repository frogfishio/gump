//! Cryptographic primitives for Capsule seal / sign / verify (DELIVERY F05),
//! release-signer trust (S02), software unseal/share ceremony (S03), and
//! HSM/KMS unseal provider trait (S04).
//!
//! Authority: docs/v1/FORMATS.md §7–§9, docs/v1/SECURITY.md §4–§6, DECISIONS D004–D005.

#![forbid(unsafe_code)]

mod aead;
mod error;
mod fingerprint;
mod hpke_wrap;
mod provider;
mod sign;
mod transcript;
mod trust;
mod unseal;

pub use aead::{
    DEK_LEN, Dek, NONCE_LEN, ProtectedPlaintext, TAG_LEN, open_protected, seal_protected,
};
pub use error::{CryptoError, CryptoErrorKind};
pub use fingerprint::{ed25519_fingerprint, format_blake3_hex, parse_blake3_hex};
pub use hpke_wrap::{
    ClusterX25519Public, ClusterX25519Secret, HPKE_SUITE_ID, SealedDek, generate_x25519_keypair,
    open_dek, seal_dek,
};
pub use provider::{
    FakeHsmUnsealProvider, SoftwareUnsealProvider, UnsealProvider, UnsealProviderDescriptor,
    UnsealProviderError, seal_and_unwrap_via_provider,
};
pub use sign::{
    SIGNATURE_LEN, SIGNING_SUITE, SigningKeyBytes, VerifyingKeyBytes, generate_signing_key,
    sign_transcript, signer_fingerprint, verify_transcript, verifying_key,
};
pub use transcript::{
    PROTECTED_AAD_PREFIX, RELEASE_SIG_PREFIX, SegmentDigestRef, build_protected_aad,
    build_release_signing_transcript, hpke_info,
};
pub use trust::{SignerEnrollment, SignerTrustPolicy, TrustCheck, TrustDecision, TrustError};
pub use unseal::{
    CLUSTER_UNSEAL_INFO, DEFAULT_SHARE_COUNT, DEFAULT_THRESHOLD, OperatorShare,
    RECOVERY_SECRET_LEN, RecoverySecret, combine_recovery_shares, derive_cluster_unseal_keypair,
    generate_recovery_secret, split_recovery_secret,
};
