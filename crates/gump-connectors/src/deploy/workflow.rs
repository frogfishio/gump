//! Deploy workflow orchestration (PROTOCOL.md §13 steps 5–6 + wait/retry).

use gump_types::{CapsuleId, WorkloadId};

use crate::deploy::idempotency::{IdempotencyCache, IdempotencyRecord};
use crate::deploy::receipt::{DeployReceipt, DurabilityGuarantee, ExecutionStatus};
use crate::deploy::types::{
    ConvergenceSnapshot, DeployFailure, DeployOutcome, DeployPhase, ObjectLocator, OrphanCapsule,
    WorkloadContract,
};
use crate::deploy::wait::{default_wait_condition, WaitCondition};

/// Backend effects for one deploy attempt (testable without live cluster).
pub trait DeployBackend {
    fn publish_capsule(
        &mut self,
        capsule_id: CapsuleId,
        capsule_digest: [u8; 32],
        operation_id: [u8; 16],
    ) -> Result<ObjectLocator, DeployFailure>;

    fn accept_intent(
        &mut self,
        operation_id: [u8; 16],
        capsule_id: CapsuleId,
        capsule_digest: [u8; 32],
    ) -> Result<(WorkloadId, u64, u64), DeployFailure>;

    fn observe(
        &mut self,
        wait: WaitCondition,
        workload_id: WorkloadId,
        generation: u64,
    ) -> Result<ConvergenceSnapshot, DeployFailure>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeployRequest {
    pub operation_id: [u8; 16],
    pub principal: String,
    pub contract: WorkloadContract,
    pub capsule_id: CapsuleId,
    pub capsule_digest: [u8; 32],
    /// Explicit wait override; `None` uses D014 defaults.
    pub wait: Option<WaitCondition>,
    pub memory_members: u32,
}

/// Orchestrates publish → accept → wait with idempotent retry and orphan report.
#[derive(Debug, Default)]
pub struct DeployWorkflow {
    pub idempotency: IdempotencyCache,
    /// Orphans reported when publish succeeds but intent accept fails.
    pub orphans: Vec<OrphanCapsule>,
    /// Last successful receipts by operation ID (for SAME_OPERATION replay).
    receipts: std::collections::BTreeMap<[u8; 16], DeployReceipt>,
}

impl DeployWorkflow {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request_digest(req: &DeployRequest) -> [u8; 32] {
        let mut buf = Vec::new();
        buf.extend_from_slice(&req.operation_id);
        buf.extend_from_slice(req.principal.as_bytes());
        buf.push(0);
        buf.extend_from_slice(req.contract.app_name.as_bytes());
        buf.push(0);
        buf.push(u8::from(req.contract.lifecycle_finite));
        buf.push(u8::from(req.contract.declares_readiness));
        buf.push(u8::from(req.contract.requires_publication));
        buf.push(u8::from(req.contract.is_gang));
        buf.extend_from_slice(&req.contract.units.to_be_bytes());
        buf.extend_from_slice(req.capsule_id.as_bytes());
        buf.extend_from_slice(&req.capsule_digest);
        if let Some(w) = req.wait {
            buf.extend_from_slice(w.as_str().as_bytes());
        }
        buf.extend_from_slice(&req.memory_members.to_be_bytes());
        *blake3::hash(&buf).as_bytes()
    }

    /// Run or replay a deploy under `operation_id` (PROTOCOL.md §15).
    pub fn run<B: DeployBackend>(
        &mut self,
        backend: &mut B,
        req: DeployRequest,
        now_ms: u64,
    ) -> DeployOutcome {
        let digest = Self::request_digest(&req);
        match self
            .idempotency
            .check(&req.operation_id, &digest, now_ms)
        {
            Err(crate::deploy::idempotency::IdempotencyError::Conflict { operation_id }) => {
                return DeployOutcome::Conflict { operation_id };
            }
            Ok(Some(_)) => {
                if let Some(receipt) = self.receipts.get(&req.operation_id) {
                    return DeployOutcome::Success(receipt.clone());
                }
            }
            Ok(None) => {}
        }

        let wait = req.wait.unwrap_or_else(|| default_wait_condition(&req.contract));

        let object = match backend.publish_capsule(
            req.capsule_id,
            req.capsule_digest,
            req.operation_id,
        ) {
            Ok(o) => o,
            Err(fail) => return DeployOutcome::Failed(fail),
        };

        let (workload_id, generation, cluster_revision) = match backend.accept_intent(
            req.operation_id,
            req.capsule_id,
            req.capsule_digest,
        ) {
            Ok(v) => v,
            Err(mut fail) => {
                let orphan = OrphanCapsule {
                    capsule_id: req.capsule_id,
                    capsule_digest: req.capsule_digest,
                    object: object.clone(),
                    operation_id: req.operation_id,
                };
                self.orphans.push(orphan.clone());
                fail.orphan = Some(orphan);
                if fail.phase != DeployPhase::IntentAccept {
                    fail.phase = DeployPhase::IntentAccept;
                }
                // Do not cache failures: SAME_OPERATION retry may resume after
                // transient accept errors without creating duplicate intent.
                return DeployOutcome::Failed(fail);
            }
        };

        let snap = match backend.observe(wait, workload_id, generation) {
            Ok(s) => s,
            Err(fail) => return DeployOutcome::Failed(fail),
        };

        let execution = if wait == WaitCondition::Accepted {
            ExecutionStatus::IntentAccepted
        } else if snap.satisfied {
            ExecutionStatus::ConditionMet {
                wait,
                eligible: snap.units_eligible,
                total: snap.units_total,
            }
        } else {
            ExecutionStatus::Converging {
                eligible: snap.units_eligible,
                total: snap.units_total,
            }
        };

        // Wait loss: observation lost before condition (CLI_LIFECYCLE §9).
        if !snap.satisfied && wait != WaitCondition::Accepted {
            return DeployOutcome::Failed(DeployFailure {
                phase: DeployPhase::WaitObservation,
                reason: format!(
                    "lost observation before wait condition '{}'",
                    wait.as_str()
                ),
                orphan: None,
            });
        }

        let receipt = DeployReceipt {
            application: req.contract.app_name.clone(),
            capsule_id: req.capsule_id,
            capsule_digest: req.capsule_digest,
            capsule_object: object,
            workload_id,
            generation,
            cluster_revision: snap.cluster_revision.max(cluster_revision),
            wait,
            execution,
            durability: DurabilityGuarantee {
                memory_members: req.memory_members.max(1),
            },
            operation_id: req.operation_id,
        };

        let result_digest = *blake3::hash(format_receipt_digest(&receipt).as_bytes()).as_bytes();
        self.idempotency.put(
            req.operation_id,
            IdempotencyRecord {
                principal: req.principal.clone(),
                request_digest: digest,
                result_digest,
                cluster_revision: receipt.cluster_revision,
                recorded_at_ms: now_ms,
            },
            now_ms,
        );
        self.receipts.insert(req.operation_id, receipt.clone());
        DeployOutcome::Success(receipt)
    }
}

fn format_receipt_digest(r: &DeployReceipt) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        r.application,
        r.workload_id.to_hyphenated(),
        r.generation,
        r.cluster_revision,
        r.wait.as_str()
    )
}
