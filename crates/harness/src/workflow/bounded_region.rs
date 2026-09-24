// SPDX-License-Identifier: MIT

//! Fail-closed admission and reporting for a bounded parallel analysis region.
//!
//! The production [`DynamicRuntime`](super::DynamicRuntime) reaches this module
//! through [`BoundedAnalysis`](super::BoundedAnalysis) so a declared adaptive
//! region can be executed on the budget-reserved bounded route instead of only
//! through a caller-supplied adaptive executor. Two properties are enforced
//! here rather than asserted by a caller:
//!
//! * **A mutation is not expressible.** [`DynamicPlan`] nodes are
//!   [`DynamicNodeKind`] values, whose vocabulary is exactly `Analyze` and
//!   `Decide`, and [`ParallelAnalysisExecutor::analyze`] returns an
//!   [`AnalysisValue`]. A plan document that names any other kind is refused by
//!   the `deny_unknown_fields` decoder before admission, so no bounded branch can
//!   reach a game mutation through this route.
//! * **Admission precedes dispatch.** [`admit_bounded_region`] validates the cap,
//!   the region's admissible operations and the plan *before* any branch is
//!   spawned, and every refusal is a typed [`BoundedRegionRefusal`].
//!
//! Reported branch states are read off the owner loop's join, never inferred
//! from the plan's shape or from UI layout.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::definition::{AdaptiveRegionConfig, WorkflowLimits};
use super::dynamic::{
    DynamicNodeKind, DynamicPlan, DynamicPlanError, ParallelAnalysisExecutor, validate_plan,
};
use super::dynamic_budget::{CancelSignal, ParallelBudget, execute_plan_bounded_reserved};
use super::dynamic_join::{BranchOutcome, JoinedResult, ParallelCap};
use super::ids::NodeId;

/// Versioned schema of the bounded-analysis report a consumer reads.
pub const BOUNDED_ANALYSIS_REPORT_SCHEMA: &str = "ascension.harness.bounded-analysis-report.v1";

/// Terminal state of one dispatched analysis branch, as the owner loop observed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", deny_unknown_fields)]
pub enum BoundedBranchState {
    #[serde(rename = "settled")]
    Settled { code: String },
    #[serde(rename = "failed")]
    Failed { reason: String },
    #[serde(rename = "unknown")]
    Unknown,
}

/// Every branch's actual state for one executed region, plus the digests that
/// bind the report to the exact plan and join.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundedAnalysisOutcome {
    pub schema: String,
    pub plan_digest: String,
    pub join_digest: String,
    pub cap: u64,
    pub branches: BTreeMap<String, BoundedBranchState>,
}

impl BoundedAnalysisOutcome {
    /// Projects the owner loop's join into the reported per-branch states.
    ///
    /// A branch that did not settle is reported as `Failed` or `Unknown` with a
    /// stable reason token; it is never folded into a settled count.
    pub fn from_joined(cap: ParallelCap, joined: &JoinedResult) -> Self {
        let branches = joined
            .outcomes
            .iter()
            .map(|(node, outcome)| (node.as_str().to_owned(), state_of(outcome)))
            .collect();
        Self {
            schema: BOUNDED_ANALYSIS_REPORT_SCHEMA.to_owned(),
            plan_digest: joined.plan_digest.as_str().to_owned(),
            join_digest: joined.join_digest.as_str().to_owned(),
            cap: u64::try_from(cap.get()).unwrap_or(u64::MAX),
            branches,
        }
    }

    /// Branches the owner loop observed as settled.
    #[must_use]
    pub fn settled(&self) -> usize {
        self.branches
            .values()
            .filter(|state| matches!(state, BoundedBranchState::Settled { .. }))
            .count()
    }

    /// Branches that did not settle, in any non-settled state.
    #[must_use]
    pub fn unsettled(&self) -> usize {
        self.branches.len().saturating_sub(self.settled())
    }
}

fn state_of(outcome: &BranchOutcome) -> BoundedBranchState {
    match outcome {
        BranchOutcome::Settled(value) => BoundedBranchState::Settled {
            code: value.code.as_str().to_owned(),
        },
        BranchOutcome::Failed(error) => BoundedBranchState::Failed {
            reason: reason_of(error).to_owned(),
        },
        BranchOutcome::Unknown => BoundedBranchState::Unknown,
    }
}

/// Stable token for a refused branch, independent of debug formatting.
#[must_use]
pub const fn reason_of(error: &DynamicPlanError) -> &'static str {
    match error {
        DynamicPlanError::InvalidPlan => "invalid_plan",
        DynamicPlanError::UnknownOperation => "unknown_operation",
        DynamicPlanError::DuplicateIdentifier => "duplicate_identifier",
        DynamicPlanError::MissingReference => "missing_reference",
        DynamicPlanError::Cycle => "cycle",
        DynamicPlanError::Capacity => "capacity",
        DynamicPlanError::BaseRevisionChanged => "base_revision_changed",
        DynamicPlanError::DependencyChanged => "dependency_changed",
        DynamicPlanError::ReplanLimit => "replan_limit",
        DynamicPlanError::NoProgress => "no_progress",
        DynamicPlanError::Persistence => "persistence",
        DynamicPlanError::UnsettledInput => "unsettled_input",
        DynamicPlanError::BranchUnknown => "branch_unknown",
        DynamicPlanError::BranchLost => "branch_lost",
        DynamicPlanError::Cancelled => "cancelled",
        DynamicPlanError::BudgetExhausted => "budget_exhausted",
    }
}

/// Typed, fail-closed refusal of a bounded analysis region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundedRegionRefusal {
    /// The workflow's parallel cap is outside the admitted `1..=4` range.
    CapOutsideAdmittedRange,
    /// The region declares no admissible analysis operation.
    RegionAdmitsNoOperations,
    /// The region's node or edge bound is not representable as a budget.
    RegionBoundsUnrepresentable,
    /// The plan is not a valid bounded plan for this region.
    PlanRejected(DynamicPlanError),
}

impl std::fmt::Display for BoundedRegionRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::CapOutsideAdmittedRange => "parallel analysis cap is outside the admitted range",
            Self::RegionAdmitsNoOperations => "adaptive region admits no analysis operation",
            Self::RegionBoundsUnrepresentable => "adaptive region bounds are not representable",
            Self::PlanRejected(_) => "bounded analysis plan was rejected for this region",
        })
    }
}

/// Admits one bounded analysis region before any branch can be dispatched.
///
/// Returns the owner's real parallel cap, so the caller cannot choose a wider
/// one than the compiled workflow limits allow.
pub fn admit_bounded_region(
    plan: &DynamicPlan,
    region: &AdaptiveRegionConfig,
    limits: &WorkflowLimits,
) -> Result<ParallelCap, BoundedRegionRefusal> {
    let cap = ParallelCap::from_limits(limits)
        .map_err(|_| BoundedRegionRefusal::CapOutsideAdmittedRange)?;
    let allowed = region
        .allowed_operations
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if allowed.is_empty() {
        return Err(BoundedRegionRefusal::RegionAdmitsNoOperations);
    }
    let max_nodes = usize::try_from(region.max_plan_nodes)
        .map_err(|_| BoundedRegionRefusal::RegionBoundsUnrepresentable)?;
    let max_edges = usize::try_from(region.max_plan_edges)
        .map_err(|_| BoundedRegionRefusal::RegionBoundsUnrepresentable)?;
    validate_plan(plan, &allowed, max_nodes, max_edges)
        .map_err(BoundedRegionRefusal::PlanRejected)?;
    Ok(cap)
}

/// Executes an admitted bounded analysis region on the budget-reserved route and
/// reports each branch's actual state.
///
/// Admission runs first, so a refused region never reaches the executor.
pub fn run_bounded_region<A: ParallelAnalysisExecutor>(
    plan: &DynamicPlan,
    region: &AdaptiveRegionConfig,
    limits: &WorkflowLimits,
    executor: &A,
    budget: &dyn ParallelBudget,
    units_per_branch: u64,
    cancel: &dyn CancelSignal,
) -> Result<(BoundedAnalysisOutcome, JoinedResult), BoundedRegionRefusal> {
    let cap = admit_bounded_region(plan, region, limits)?;
    let joined =
        execute_plan_bounded_reserved(plan, cap, executor, budget, units_per_branch, cancel)
            .map_err(BoundedRegionRefusal::PlanRejected)?;
    Ok((BoundedAnalysisOutcome::from_joined(cap, &joined), joined))
}

/// Whether a plan node kind is an analysis kind.
///
/// The bounded vocabulary is closed, so this is `true` for every value a decoded
/// [`DynamicPlan`] can hold; the refusal for anything else happens in the
/// decoder, which is why [`DynamicPlan`] is the admission input and not a looser
/// document type.
#[must_use]
pub const fn is_analysis_kind(kind: &DynamicNodeKind) -> bool {
    matches!(
        kind,
        DynamicNodeKind::Analyze { .. } | DynamicNodeKind::Decide { .. }
    )
}

/// The plan nodes that were admitted, in identity order.
pub fn admitted_nodes(plan: &DynamicPlan) -> Result<Vec<NodeId>, DynamicPlanError> {
    let mut nodes = plan
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    nodes.sort();
    nodes.dedup();
    if nodes.len() != plan.nodes.len() {
        return Err(DynamicPlanError::DuplicateIdentifier);
    }
    Ok(nodes)
}
