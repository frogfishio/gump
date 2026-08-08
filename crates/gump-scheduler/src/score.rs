//! Scoring among hard-filter survivors (RUNTIME.md §2 step 5 / R03).

use crate::capability::NodeResources;
use crate::explain::{ExplainReason, codes};
use crate::filter::with_headroom;

/// Higher score is preferred. Residual headroom after the request (+headroom)
/// favors spread / pressure avoidance for the minimum placement slice.
pub fn score_residual_headroom(
    free: NodeResources,
    request: NodeResources,
) -> (i64, ExplainReason) {
    let need = with_headroom(request);
    let left = free.saturating_sub(need);
    // Pack millicores and memory into one comparable integer (memory in MiB).
    let score = i64::from(left.millicores)
        .saturating_add((left.memory_bytes / (1024 * 1024)) as i64)
        .saturating_add(i64::from(left.gpu_devices) * 1_000);
    (
        score,
        ExplainReason::new(
            codes::SCORE_HEADROOM,
            score,
            "residual headroom after headroomed request",
        ),
    )
}
