//! Streamed Capsule ingress (D03 / SYSTEM_DESIGN §13.1.1).
//!
//! Streams exact sealed bytes into quarantine while enforcing size limits and
//! hashing. Never unseals. Structural/signature/trust verification runs while
//! quarantined; then write-if-absent publish.

use std::io::Read;

use gump_capsule::{read_gump_capsule, verify_release_signature, SegmentType};
use gump_crypto::{SignerTrustPolicy, TrustCheck, TrustError, VerifyingKeyBytes};
use gump_types::{CapsuleId, ClusterId};

use crate::object::{final_capsule_key, ObjectEvidence, ObjectStore, ObjectStoreError};

/// Default max chunk size kept in the ingest buffer (peak-memory bound).
pub const DEFAULT_MAX_CHUNK_BYTES: usize = 64 * 1024;

/// D008 single-object Capsule ceiling (5 GiB).
pub const DEFAULT_MAX_CAPSULE_BYTES: u64 = 5 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngressLimits {
    pub max_capsule_bytes: u64,
    pub max_chunk_bytes: usize,
}

impl Default for IngressLimits {
    fn default() -> Self {
        Self {
            max_capsule_bytes: DEFAULT_MAX_CAPSULE_BYTES,
            max_chunk_bytes: DEFAULT_MAX_CHUNK_BYTES,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngestStats {
    pub bytes_received: u64,
    pub peak_buffer_bytes: usize,
    pub digest: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngressReceipt {
    pub evidence: ObjectEvidence,
    pub stats: IngestStats,
    pub signer_fingerprint: String,
    pub trust_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IngressError {
    Io(String),
    Oversize {
        received: u64,
        max: u64,
    },
    EmptyBody,
    Object(ObjectStoreError),
    Capsule(String),
    Trust(TrustError),
    Signature,
    ClusterMismatch,
    CapsuleIdMismatch,
    Truncated {
        got: u64,
        expected: u64,
    },
}

impl std::fmt::Display for IngressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "ingress io: {e}"),
            Self::Oversize { received, max } => {
                write!(f, "capsule {received} exceeds max {max}")
            }
            Self::EmptyBody => write!(f, "empty ingress body"),
            Self::Object(e) => write!(f, "{e}"),
            Self::Capsule(e) => write!(f, "capsule: {e}"),
            Self::Trust(e) => write!(f, "{e}"),
            Self::Signature => write!(f, "release signature verification failed"),
            Self::ClusterMismatch => write!(f, "capsule cluster_id does not match ingress cluster"),
            Self::CapsuleIdMismatch => {
                write!(f, "capsule_id does not match ingress assignment")
            }
            Self::Truncated { got, expected } => {
                write!(f, "truncated body: got {got}, expected {expected}")
            }
        }
    }
}

impl std::error::Error for IngressError {}

impl From<ObjectStoreError> for IngressError {
    fn from(e: ObjectStoreError) -> Self {
        Self::Object(e)
    }
}

impl From<TrustError> for IngressError {
    fn from(e: TrustError) -> Self {
        Self::Trust(e)
    }
}

/// Ingress role: stream → quarantine → verify → publish_if_absent (no unseal).
#[derive(Clone, Debug)]
pub struct StreamedIngress {
    pub limits: IngressLimits,
}

impl Default for StreamedIngress {
    fn default() -> Self {
        Self {
            limits: IngressLimits::default(),
        }
    }
}

impl StreamedIngress {
    pub fn new(limits: IngressLimits) -> Self {
        Self { limits }
    }

    /// Stream a body of known `content_length` (HTTP Content-Length) into quarantine,
    /// verify while quarantined, then promote with write-if-absent.
    ///
    /// Peak buffer during ingest is at most `limits.max_chunk_bytes`.
    pub fn accept_known_length<S: ObjectStore>(
        &self,
        store: &mut S,
        trust: &SignerTrustPolicy,
        cluster: ClusterId,
        capsule: CapsuleId,
        namespace: &str,
        now_ms: u64,
        content_length: u64,
        reader: &mut dyn Read,
    ) -> Result<IngressReceipt, IngressError> {
        if content_length == 0 {
            return Err(IngressError::EmptyBody);
        }
        if content_length > self.limits.max_capsule_bytes {
            return Err(IngressError::Oversize {
                received: content_length,
                max: self.limits.max_capsule_bytes,
            });
        }

        let max_chunk = self.limits.max_chunk_bytes.max(1);
        let mut buf = vec![0u8; max_chunk];
        let mut hasher = blake3::Hasher::new();
        let mut received = 0u64;
        let mut peak = 0usize;

        let upload = store.begin_quarantine(cluster, capsule, content_length)?;

        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| IngressError::Io(e.to_string()))?;
            if n == 0 {
                break;
            }
            peak = peak.max(n);
            let next = received.saturating_add(n as u64);
            if next > content_length {
                let _ = store.abort(upload);
                return Err(IngressError::Oversize {
                    received: next,
                    max: content_length,
                });
            }
            hasher.update(&buf[..n]);
            if let Err(e) = store.write(upload, &buf[..n]) {
                let _ = store.abort(upload);
                return Err(e.into());
            }
            received = next;
        }

        if received != content_length {
            let _ = store.abort(upload);
            return Err(IngressError::Truncated {
                got: received,
                expected: content_length,
            });
        }

        let digest = *hasher.finalize().as_bytes();
        let q = store.finish_quarantine(upload, digest)?;

        let bytes = store.get(&q.key, None)?;
        let view =
            read_gump_capsule(&bytes).map_err(|e| IngressError::Capsule(e.to_string()))?;
        if &view.header.cluster_id != cluster.as_bytes() {
            let _ = store.delete(&q.key);
            return Err(IngressError::ClusterMismatch);
        }
        if &view.header.capsule_id != capsule.as_bytes() {
            let _ = store.delete(&q.key);
            return Err(IngressError::CapsuleIdMismatch);
        }

        let sig_seg = view.segment(SegmentType::ReleaseSignature);
        if sig_seg.len() < 96 {
            let _ = store.delete(&q.key);
            return Err(IngressError::Signature);
        }
        let mut vk = [0u8; 32];
        let mut signature = [0u8; 64];
        vk.copy_from_slice(&sig_seg[..32]);
        signature.copy_from_slice(&sig_seg[32..96]);

        let header_cbor = view
            .header
            .encode_cbor()
            .map_err(|e| IngressError::Capsule(e.to_string()))?;
        verify_release_signature(&header_cbor, &view.table, &vk, &signature)
            .map_err(|_| IngressError::Signature)?;

        let trust_decision = trust.check(
            &VerifyingKeyBytes(vk),
            namespace,
            now_ms,
            TrustCheck::Publication,
            None,
        )?;

        let final_key = final_capsule_key(cluster, capsule)?;
        let evidence = store.publish_if_absent(&q.key, &final_key, digest, received)?;
        let _ = store.delete(&q.key);

        Ok(IngressReceipt {
            evidence,
            stats: IngestStats {
                bytes_received: received,
                peak_buffer_bytes: peak,
                digest,
            },
            signer_fingerprint: trust_decision.fingerprint,
            trust_revision: trust_decision.policy_revision,
        })
    }
}
