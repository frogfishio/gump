//! Hard-filter stage (RUNTIME.md §2 steps 3–4). Scores never bypass these.

use crate::capability::{CapabilityReport, NodeResources, WorkloadRequirements};
use crate::explain::{ExplainReason, codes};

/// Conservative headroom on top of declared request (RUNTIME.md §2).
pub fn with_headroom(request: NodeResources) -> NodeResources {
    let millicores = request.millicores.saturating_add(
        request
            .millicores
            .div_ceil(5)
            .max(if request.millicores > 0 { 1 } else { 0 }),
    );
    let memory_bytes = request.memory_bytes.saturating_add(
        request
            .memory_bytes
            .div_ceil(5)
            .max(if request.memory_bytes > 0 { 1 } else { 0 }),
    );
    NodeResources {
        millicores,
        memory_bytes,
        gpu_devices: request.gpu_devices,
        ports: request.ports,
    }
}

/// Evaluate hard requirements against a node report and free resources.
///
/// Returns every failed hard requirement (unschedulable output must list them all).
pub fn hard_filter(
    req: &WorkloadRequirements,
    report: &CapabilityReport,
    free: NodeResources,
) -> Result<(), Vec<ExplainReason>> {
    let mut reasons = Vec::new();

    if report.drained {
        reasons.push(ExplainReason::new(
            codes::NODE_DRAINED,
            1,
            "node is drained",
        ));
    }
    if report.arch != req.arch {
        reasons.push(ExplainReason::new(
            codes::ARCH_MISMATCH,
            0,
            format!("need arch {} have {}", req.arch, report.arch),
        ));
    }
    if !report.driver_supported(&req.driver) {
        reasons.push(ExplainReason::new(
            codes::DRIVER_MISSING,
            0,
            format!("driver {} not on node", req.driver),
        ));
    }
    for name in &req.required_enforced {
        match report.capability_level(name) {
            None => reasons.push(ExplainReason::new(
                codes::CAPABILITY_MISSING,
                0,
                format!("capability {name} absent"),
            )),
            Some(level) if level.satisfies_enforcement() => {}
            Some(level) => reasons.push(ExplainReason::new(
                codes::CAPABILITY_NOT_ENFORCED,
                0,
                format!("capability {name} is {} (need enforced)", level.as_str()),
            )),
        }
    }

    let need = with_headroom(req.request);
    if free.millicores < need.millicores {
        reasons.push(ExplainReason::new(
            codes::MILLICORES,
            i64::from(free.millicores),
            format!("need {} millicores (with headroom)", need.millicores),
        ));
    }
    if free.memory_bytes < need.memory_bytes {
        reasons.push(ExplainReason::new(
            codes::MEMORY,
            free.memory_bytes as i64,
            format!("need {} memory bytes (with headroom)", need.memory_bytes),
        ));
    }
    if free.gpu_devices < need.gpu_devices {
        reasons.push(ExplainReason::new(
            codes::GPU,
            i64::from(free.gpu_devices),
            format!("need {} GPU devices", need.gpu_devices),
        ));
    }
    if req.requires_port && free.ports < 1 {
        reasons.push(ExplainReason::new(
            codes::PORT_REQUIRED,
            i64::from(free.ports),
            "workload requires a host port; node has none free",
        ));
    }

    if reasons.is_empty() {
        Ok(())
    } else {
        Err(reasons)
    }
}
