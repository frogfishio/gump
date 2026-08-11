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
    AcceptResult, DECLARATION_SCHEMA, DECLARATION_SIG_DOMAIN, DeclarationDraft, DeclarationError,
    DeclarationLedger, NormalizedDeclaration, OverrideProvenance, normalize_declaration,
    sign_declaration, verify_declaration_signature,
};
pub use deploy::{
    ConvergenceSnapshot, DeployBackend, DeployFailure, DeployOutcome, DeployPhase, DeployReceipt,
    DeployRequest, DeployWorkflow, DurabilityGuarantee, ExecutionStatus, IdempotencyCache,
    IdempotencyError, IdempotencyRecord, ObjectLocator, OrphanCapsule, WaitCondition,
    WorkloadContract, default_wait_condition, format_receipt_human,
};
pub use ingress::{
    DEFAULT_MAX_CAPSULE_BYTES, DEFAULT_MAX_CHUNK_BYTES, IngestStats, IngressError, IngressLimits,
    IngressReceipt, StreamedIngress,
};
pub use object::{
    ByteRange, FakeObjectStore, META_BLAKE3, ObjectEvidence, ObjectKey, ObjectStore,
    ObjectStoreError, ObjectStoreErrorKind, RuntimeObjectStore, S3Config, S3ObjectStore,
    S3ReadStats, UploadId, UploadProgress, final_capsule_key, is_final_capsule_key,
    parse_final_capsule_key, quarantine_key,
};
