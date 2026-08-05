//! Connector contracts (DELIVERY D01–D05+).
//!
//! Object-store connectors hold Capsule bytes only. Streamed ingress (D03)
//! verifies sealed Capsules without unsealing. Declarations (D04) bind intent
//! in memory without Capsule bytes or runtime plaintext. Deploy workflow (D05)
//! produces truthful receipts, waits, retries, and orphan reports.

#![forbid(unsafe_code)]

pub mod declaration;
pub mod deploy;
pub mod ingress;
pub mod object;

pub use declaration::{
    normalize_declaration, sign_declaration, verify_declaration_signature, AcceptResult,
    DeclarationDraft, DeclarationError, DeclarationLedger, NormalizedDeclaration,
    OverrideProvenance, DECLARATION_SCHEMA, DECLARATION_SIG_DOMAIN,
};
pub use deploy::{
    default_wait_condition, format_receipt_human, ConvergenceSnapshot, DeployBackend,
    DeployFailure, DeployOutcome, DeployPhase, DeployReceipt, DeployRequest, DeployWorkflow,
    DurabilityGuarantee, ExecutionStatus, IdempotencyCache, IdempotencyError, IdempotencyRecord,
    ObjectLocator, OrphanCapsule, WaitCondition, WorkloadContract,
};
pub use ingress::{
    IngestStats, IngressError, IngressLimits, IngressReceipt, StreamedIngress,
    DEFAULT_MAX_CAPSULE_BYTES, DEFAULT_MAX_CHUNK_BYTES,
};
pub use object::{
    final_capsule_key, quarantine_key, ByteRange, FakeObjectStore, ObjectEvidence, ObjectKey,
    ObjectStore, ObjectStoreError, ObjectStoreErrorKind, S3Config, S3ObjectStore, UploadId,
    UploadProgress,
};
