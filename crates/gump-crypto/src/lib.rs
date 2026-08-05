//! Cryptographic primitives for Capsule seal / sign / verify (DELIVERY F05),
//! release-signer trust (S02), and software unseal/share ceremony (S03).
//!
//! Authority: docs/v1/FORMATS.md §7–§9, docs/v1/SECURITY.md §4–§6, DECISIONS D004–D005.

#![forbid(unsafe_code)]

mod aead;
mod error;
mod fingerprint;
mod hpke_wrap;
mod sign;
mod transcript;
mod trust;
mod unseal;

pub use aead::{open_protected, seal_protected, DEK_LEN, NONCE_LEN, TAG_LEN};
pub use error::{CryptoError, CryptoErrorKind};
pub use fingerprint::{ed25519_fingerprint, format_blake3_hex, parse_blake3_hex};
pub use hpke_wrap::{
    generate_x25519_keypair, open_dek, seal_dek, ClusterX25519Public, ClusterX25519Secret,
    SealedDek, HPKE_SUITE_ID,
};
pub use sign::{
    generate_signing_key, sign_transcript, signer_fingerprint, verifying_key, verify_transcript,
    SigningKeyBytes, VerifyingKeyBytes, SIGNATURE_LEN, SIGNING_SUITE,
};
pub use transcript::{
    build_protected_aad, build_release_signing_transcript, hpke_info, SegmentDigestRef,
    PROTECTED_AAD_PREFIX, RELEASE_SIG_PREFIX,
};
pub use trust::{
    SignerEnrollment, SignerTrustPolicy, TrustCheck, TrustDecision, TrustError,
};
pub use unseal::{
    combine_recovery_shares, derive_cluster_unseal_keypair, generate_recovery_secret,
    split_recovery_secret, OperatorShare, RecoverySecret, CLUSTER_UNSEAL_INFO,
    DEFAULT_SHARE_COUNT, DEFAULT_THRESHOLD, RECOVERY_SECRET_LEN,
};
