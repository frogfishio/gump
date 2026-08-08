//! Deploy receipt stages and wait defaults (GUMP-N015 / D05 / CONFORMANCE §6).

use super::machine::{DeployStageBody, DeployWaitBody};

/// Default wait for deploy when the workload contract does not declare otherwise.
pub const DEFAULT_DEPLOY_WAIT: &str = "intent_accepted";

/// Normalize a client `--wait` value; unknown names fall back to the default.
pub fn normalize_wait_condition(requested: Option<&str>) -> String {
    match requested.map(str::trim).filter(|s| !s.is_empty()) {
        None => DEFAULT_DEPLOY_WAIT.into(),
        Some("intent_accepted") | Some("accepted") => DEFAULT_DEPLOY_WAIT.into(),
        Some("scheduled") => "scheduled".into(),
        Some("started") | Some("start") => "started".into(),
        Some("ready") | Some("readiness") => "readiness".into(),
        Some("published") | Some("publication") => "publication".into(),
        Some("completed") | Some("completion") => "completion".into(),
        Some(other) => other.to_string(),
    }
}

pub fn wait_body(requested: Option<&str>) -> DeployWaitBody {
    let condition = normalize_wait_condition(requested);
    DeployWaitBody {
        condition: condition.clone(),
        default_for_contract: DEFAULT_DEPLOY_WAIT.into(),
        matched_default: condition == DEFAULT_DEPLOY_WAIT,
    }
}

/// Build stage checklist for a successful intent-accept receipt.
///
/// Persistence + intent acceptance are completed; later lifecycle stages remain
/// `pending` / `not_observed` until the control plane reports them. Observation
/// loss is never silently omitted.
pub fn intent_accepted_stages(replayed: bool) -> Vec<DeployStageBody> {
    let persist_note = if replayed {
        "capsule already published; receipt replayed from cluster memory"
    } else {
        "sealed Capsule published to object store"
    };
    vec![
        stage("persistence", "completed", persist_note),
        stage(
            "intent_acceptance",
            "completed",
            if replayed {
                "Idempotent PutDesired replay from Raft"
            } else {
                "live desired intent committed in cluster memory"
            },
        ),
        stage("scheduling", "pending", "placement not yet observed"),
        stage("start", "pending", "execution start not yet observed"),
        stage(
            "readiness",
            "not_observed",
            "readiness never inferred when undeclared",
        ),
        stage(
            "publication",
            "not_observed",
            "publication never inferred when undeclared",
        ),
        stage(
            "completion",
            "pending",
            "workload completion not yet observed",
        ),
        stage(
            "observation",
            "available",
            "lossy; interruption or deadline does not imply rollback",
        ),
    ]
}

fn stage(name: &str, status: &str, detail: &str) -> DeployStageBody {
    DeployStageBody {
        name: name.into(),
        status: status.into(),
        detail: detail.into(),
    }
}
