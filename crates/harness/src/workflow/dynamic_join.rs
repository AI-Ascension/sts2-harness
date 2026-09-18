// SPDX-License-Identifier: MIT

//! Join vocabulary for the bounded dynamic-analysis route: the owner-enforced
//! in-flight cap, per-branch outcomes, and the identity-ordered joined result
//! whose digest is independent of completion timing.

use std::collections::BTreeMap;

use serde::Serialize;

use super::definition::WorkflowLimits;
use super::dynamic::{DynamicPlanError, DynamicPlanResult};
use super::ids::{Digest, NodeId};
use super::values::AnalysisValue;

/// Upper bound accepted for `WorkflowLimits::max_parallel_analyses`.
pub const MAX_PARALLEL_ANALYSES: u64 = 4;

/// Owner-enforced number of analysis branches that may be in flight at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParallelCap(usize);

impl ParallelCap {
    /// The cap=1 compatibility route: one branch at a time, in topological order.
    pub const SERIAL: Self = Self(1);

    pub fn new(cap: u64) -> Result<Self, DynamicPlanError> {
        if cap == 0 || cap > MAX_PARALLEL_ANALYSES {
            return Err(DynamicPlanError::Capacity);
        }
        usize::try_from(cap)
            .map(Self)
            .map_err(|_| DynamicPlanError::Capacity)
    }

    pub fn from_limits(limits: &WorkflowLimits) -> Result<Self, DynamicPlanError> {
        Self::new(limits.max_parallel_analyses)
    }

    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

/// Terminal state of one plan node after the bounded route ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchOutcome {
    Settled(AnalysisValue),
    Failed(DynamicPlanError),
    Unknown,
}

impl BranchOutcome {
    #[must_use]
    pub const fn is_settled(&self) -> bool {
        matches!(self, Self::Settled(_))
    }

    pub(super) fn settled_value(&self) -> Option<&AnalysisValue> {
        match self {
            Self::Settled(value) => Some(value),
            Self::Failed(_) | Self::Unknown => None,
        }
    }

    fn compatibility_error(&self) -> Option<DynamicPlanError> {
        match self {
            Self::Settled(_) => None,
            Self::Failed(error) => Some(error.clone()),
            Self::Unknown => Some(DynamicPlanError::BranchUnknown),
        }
    }
}

/// Every node's outcome, joined by node identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinedResult {
    pub plan_digest: Digest,
    pub outcomes: BTreeMap<NodeId, BranchOutcome>,
    pub join_digest: Digest,
}

impl JoinedResult {
    /// Projects the join onto the serial route's result type.
    ///
    /// Succeeds only when every node settled; otherwise the first non-settled
    /// node in identity order names the error, so a failed or unknown branch can
    /// never be read as a successful plan result.
    pub fn into_plan_result(self) -> Result<DynamicPlanResult, DynamicPlanError> {
        let mut analyses = BTreeMap::new();
        for (id, outcome) in self.outcomes {
            if let Some(error) = outcome.compatibility_error() {
                return Err(error);
            }
            if let BranchOutcome::Settled(value) = outcome {
                analyses.insert(id, value);
            }
        }
        Ok(DynamicPlanResult {
            plan_digest: self.plan_digest,
            analyses,
        })
    }
}

#[derive(Serialize)]
struct JoinEntry<'a> {
    node: &'a str,
    state: &'static str,
    value: Option<&'a AnalysisValue>,
    error: Option<String>,
}

#[derive(Serialize)]
struct JoinRecord<'a> {
    plan_digest: &'a str,
    entries: Vec<JoinEntry<'a>>,
}

pub(super) fn join_digest(
    plan_digest: &Digest,
    outcomes: &BTreeMap<NodeId, BranchOutcome>,
) -> Result<Digest, DynamicPlanError> {
    let entries = outcomes
        .iter()
        .map(|(id, outcome)| JoinEntry {
            node: id.as_str(),
            state: match outcome {
                BranchOutcome::Settled(_) => "settled",
                BranchOutcome::Failed(_) => "failed",
                BranchOutcome::Unknown => "unknown",
            },
            value: outcome.settled_value(),
            error: match outcome {
                BranchOutcome::Failed(error) => Some(error.to_string()),
                BranchOutcome::Settled(_) | BranchOutcome::Unknown => None,
            },
        })
        .collect();
    let record = JoinRecord {
        plan_digest: plan_digest.as_str(),
        entries,
    };
    let bytes = serde_json::to_vec(&record).map_err(|_| DynamicPlanError::InvalidPlan)?;
    Ok(Digest::sha256(&bytes))
}
