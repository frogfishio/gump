//! Scoped secret delivery authorization (S07 / INV-013 / GUMP-N009).
//!
//! Agents bind plaintext values to an exact delivery scope after placement
//! admission. Wrong node/release/unit/attempt/fence/epoch replays fail closed
//! before the driver injects anything.

use core::fmt;

use gump_driver::{DeliveryScope, InjectForm, SecretPlan, SecretValue};
use gump_types::Secret;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeliveryError {
    ScopeMismatch { field: &'static str },
    EmptyValues,
}

impl fmt::Display for DeliveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ScopeMismatch { field } => {
                write!(f, "secret delivery scope mismatch on {field}")
            }
            Self::EmptyValues => write!(f, "secret delivery requires at least one value"),
        }
    }
}

impl std::error::Error for DeliveryError {}

/// Fail closed unless every scope field matches the live authorized attempt.
pub fn authorize_delivery(
    requested: &DeliveryScope,
    live: &DeliveryScope,
) -> Result<(), DeliveryError> {
    if requested.cluster_id != live.cluster_id {
        return Err(DeliveryError::ScopeMismatch {
            field: "cluster_id",
        });
    }
    if requested.workload_id != live.workload_id {
        return Err(DeliveryError::ScopeMismatch {
            field: "workload_id",
        });
    }
    if requested.release_id != live.release_id {
        return Err(DeliveryError::ScopeMismatch {
            field: "release_id",
        });
    }
    if requested.unit != live.unit {
        return Err(DeliveryError::ScopeMismatch { field: "unit" });
    }
    if requested.attempt_id != live.attempt_id {
        return Err(DeliveryError::ScopeMismatch {
            field: "attempt_id",
        });
    }
    if requested.node_id != live.node_id {
        return Err(DeliveryError::ScopeMismatch { field: "node_id" });
    }
    if requested.controller_epoch != live.controller_epoch {
        return Err(DeliveryError::ScopeMismatch {
            field: "controller_epoch",
        });
    }
    if requested.placement_fence != live.placement_fence {
        return Err(DeliveryError::ScopeMismatch {
            field: "placement_fence",
        });
    }
    Ok(())
}

/// Build a driver [`SecretPlan`] only after live scope authorization.
pub fn bind_secret_plan(
    requested: DeliveryScope,
    live: &DeliveryScope,
    values: Vec<(String, InjectForm, Secret<Vec<u8>>)>,
) -> Result<SecretPlan, DeliveryError> {
    authorize_delivery(&requested, live)?;
    if values.is_empty() {
        return Err(DeliveryError::EmptyValues);
    }
    let values = values
        .into_iter()
        .map(|(logical_name, form, bytes)| SecretValue {
            logical_name,
            form,
            bytes,
        })
        .collect();
    Ok(SecretPlan::scoped(requested, values))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gump_types::{AttemptId, CapsuleId, ClusterId, WorkloadId};

    fn scope(tag: u8) -> DeliveryScope {
        let mut id = [
            0x01, 0x8f, 0x4a, 0x00, 0x00, 0x00, 0x70, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        id[15] = tag;
        DeliveryScope {
            cluster_id: ClusterId::from_bytes(id).unwrap(),
            workload_id: WorkloadId::from_bytes({
                let mut w = id;
                w[14] = 1;
                w
            })
            .unwrap(),
            release_id: CapsuleId::from_bytes({
                let mut r = id;
                r[14] = 2;
                r
            })
            .unwrap(),
            unit: 0,
            attempt_id: AttemptId::from_bytes({
                let mut a = id;
                a[14] = 3;
                a
            })
            .unwrap(),
            node_id: 1,
            controller_epoch: 7,
            placement_fence: 9,
        }
    }

    #[test]
    fn authorize_rejects_wrong_fence_and_attempt() {
        let live = scope(1);
        let mut bad = live.clone();
        bad.placement_fence = 99;
        assert!(matches!(
            authorize_delivery(&bad, &live),
            Err(DeliveryError::ScopeMismatch {
                field: "placement_fence"
            })
        ));
        let mut bad = live.clone();
        bad.attempt_id = AttemptId::new();
        assert!(matches!(
            authorize_delivery(&bad, &live),
            Err(DeliveryError::ScopeMismatch {
                field: "attempt_id"
            })
        ));
    }

    #[test]
    fn bind_plan_succeeds_only_for_exact_scope() {
        let live = scope(2);
        let canary = Secret::new(b"n009-canary-SECRET".to_vec());
        let plan = bind_secret_plan(
            live.clone(),
            &live,
            vec![("TOKEN".into(), InjectForm::Env, canary)],
        )
        .unwrap();
        assert!(!plan.deferred);
        assert_eq!(plan.values.len(), 1);
        // Debug must not echo canary.
        assert!(!format!("{plan:?}").contains("n009-canary"));

        let mut wrong_node = live.clone();
        wrong_node.node_id = 2;
        let err = bind_secret_plan(
            wrong_node,
            &live,
            vec![("TOKEN".into(), InjectForm::Env, Secret::new(b"x".to_vec()))],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            DeliveryError::ScopeMismatch { field: "node_id" }
        ));
        assert!(!err.to_string().contains("n009-canary"));
    }
}
