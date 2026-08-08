//! Placement pipeline: filter → score → atomic reserve → fence admit (R02–R04).

use std::collections::BTreeMap;

use gump_types::NodeId;

use crate::capability::{CapabilityReport, WorkloadRequirements};
use crate::explain::{ExplainReason, codes};
use crate::filter::hard_filter;
use crate::ledger::{Reservation, ResourceLedger};
use crate::score::score_residual_headroom;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeFeasibility {
    pub node_id: NodeId,
    pub feasible: bool,
    pub reasons: Vec<ExplainReason>,
    pub score: Option<i64>,
    pub score_explain: Option<ExplainReason>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlacementPlan {
    pub reservation: Reservation,
    pub score: i64,
    pub score_explain: ExplainReason,
    /// Per-node feasibility matrix (every rejection listed).
    pub matrix: Vec<NodeFeasibility>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlacementOutcome {
    Scheduled(PlacementPlan),
    Unschedulable {
        matrix: Vec<NodeFeasibility>,
        summary: ExplainReason,
    },
}

/// In-memory placement controller: capability corpus + resource ledger.
#[derive(Debug, Default)]
pub struct PlacementController {
    reports: BTreeMap<NodeId, CapabilityReport>,
    pub ledger: ResourceLedger,
    max_reports: usize,
}

impl PlacementController {
    pub const DEFAULT_MAX_REPORTS: usize = 4_096;

    pub fn new() -> Self {
        Self {
            reports: BTreeMap::new(),
            ledger: ResourceLedger::new(),
            max_reports: Self::DEFAULT_MAX_REPORTS,
        }
    }

    pub fn report_count(&self) -> usize {
        self.reports.len()
    }

    /// Upsert a capability report (bounded; oldest NodeId evicted on ceiling).
    pub fn upsert_report(&mut self, report: CapabilityReport) -> Result<(), ExplainReason> {
        if !self.reports.contains_key(&report.node_id) && self.reports.len() >= self.max_reports {
            if let Some(oldest) = self.reports.keys().next().copied() {
                self.reports.remove(&oldest);
            }
        }
        if !self.reports.contains_key(&report.node_id) && self.reports.len() >= self.max_reports {
            return Err(ExplainReason::new(
                codes::LEDGER_FULL,
                self.max_reports as i64,
                "capability report ceiling reached",
            ));
        }
        self.reports.insert(report.node_id, report);
        Ok(())
    }

    pub fn report(&self, node: NodeId) -> Option<&CapabilityReport> {
        self.reports.get(&node)
    }

    /// Run hard-filter + score + atomic reserve for one independent unit.
    pub fn place(&mut self, req: &WorkloadRequirements) -> PlacementOutcome {
        let mut req = req.clone();
        if req.requires_port && req.request.ports == 0 {
            req.request.ports = 1;
        }

        let mut matrix = Vec::new();
        let mut best: Option<(NodeId, i64, ExplainReason)> = None;

        for report in self.reports.values() {
            let free = self.ledger.free_on(report.node_id, report.allocatable);
            match hard_filter(&req, report, free) {
                Ok(()) => {
                    let (score, score_explain) = score_residual_headroom(free, req.request);
                    matrix.push(NodeFeasibility {
                        node_id: report.node_id,
                        feasible: true,
                        reasons: Vec::new(),
                        score: Some(score),
                        score_explain: Some(score_explain.clone()),
                    });
                    let take = match &best {
                        None => true,
                        Some((nid, best_score, _)) => {
                            score > *best_score
                                || (score == *best_score
                                    && report.node_id.to_hyphenated() < nid.to_hyphenated())
                        }
                    };
                    if take {
                        best = Some((report.node_id, score, score_explain));
                    }
                }
                Err(reasons) => matrix.push(NodeFeasibility {
                    node_id: report.node_id,
                    feasible: false,
                    reasons,
                    score: None,
                    score_explain: None,
                }),
            }
        }

        // Stable matrix order by node id.
        matrix.sort_by_key(|n| n.node_id.to_hyphenated());

        let Some((node_id, score, score_explain)) = best else {
            return PlacementOutcome::Unschedulable {
                matrix,
                summary: ExplainReason::new(codes::NO_CANDIDATE, 0, "no node passed hard filters"),
            };
        };

        let report = match self.reports.get(&node_id) {
            Some(r) => r.clone(),
            None => {
                return PlacementOutcome::Unschedulable {
                    matrix,
                    summary: ExplainReason::new(codes::NO_CANDIDATE, 0, "selected node vanished"),
                };
            }
        };

        match self.ledger.reserve(
            req.unit_id,
            node_id,
            req.request,
            report.allocatable,
            report.revision,
            report.placement_fence,
        ) {
            Ok(reservation) => PlacementOutcome::Scheduled(PlacementPlan {
                reservation,
                score,
                score_explain,
                matrix,
            }),
            Err(reason) => {
                matrix.iter_mut().for_each(|row| {
                    if row.node_id == node_id {
                        row.feasible = false;
                        row.reasons.push(reason.clone());
                        row.score = None;
                        row.score_explain = None;
                    }
                });
                PlacementOutcome::Unschedulable {
                    matrix,
                    summary: reason,
                }
            }
        }
    }

    /// Agent admission against current local capability facts (RUNTIME.md §2 step 7).
    ///
    /// Fails closed on stale capability revision or stale placement fence.
    pub fn admit(
        &self,
        reservation: &Reservation,
        live: &CapabilityReport,
    ) -> Result<(), Vec<ExplainReason>> {
        let mut reasons = Vec::new();
        if live.node_id != reservation.node_id {
            reasons.push(ExplainReason::new(
                codes::STALE_CAPABILITY,
                0,
                "admit node_id does not match reservation",
            ));
        }
        if live.revision != reservation.capability_revision {
            reasons.push(ExplainReason::new(
                codes::STALE_CAPABILITY,
                live.revision as i64,
                format!(
                    "capability revision {} != reserved {}",
                    live.revision, reservation.capability_revision
                ),
            ));
        }
        if live.placement_fence != reservation.placement_fence {
            reasons.push(ExplainReason::new(
                codes::STALE_FENCE,
                live.placement_fence as i64,
                format!(
                    "placement fence {} != reserved {}",
                    live.placement_fence, reservation.placement_fence
                ),
            ));
        }
        if reasons.is_empty() {
            Ok(())
        } else {
            Err(reasons)
        }
    }
}
