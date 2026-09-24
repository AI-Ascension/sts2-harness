// SPDX-License-Identifier: MIT

//! Versioned, bounded declarations of a same-start branch experiment.
//!
//! One experiment names a single verified fork point, a fork strategy, the child policies or
//! explicit first-action alternatives, the stop conditions and the per-child/total budgets. The
//! declaration is private input: it is never a claim that any child was admitted, restored or run.

use std::collections::BTreeSet;

use crate::{ExactCheckpointReference, OccurrenceId, ProjectionError, ProjectionKey, sha256_hex};

use super::error::BranchExperimentError;
use super::label::{child_label_ok, digest_ok, label_ok};

/// Only supported branch-experiment declaration version.
pub const BRANCH_EXPERIMENT_VERSION: &str = "ascension.branch-experiment.v1";
/// Maximum children in one experiment.
pub const MAX_BRANCH_CHILDREN: usize = 64;
/// Whole-declaration bound, checked before deriving a digest.
pub const MAX_MANIFEST_BYTES: usize = 32 * 1024;
/// Maximum provider/context concurrency one experiment may declare.
pub const MAX_BRANCH_CONCURRENCY: u32 = 32;

/// How the first action of each child is chosen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForkStrategy {
    /// Every child is handed an explicit alternative first action.
    AlternativeFirstAction,
    /// Every child chooses its own first action under its policy settings.
    AlternatePolicy,
}

impl ForkStrategy {
    /// Returns the stable lowercase label of this strategy.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::AlternativeFirstAction => "alternative_first_action",
            Self::AlternatePolicy => "alternate_policy",
        }
    }
}

/// A declared reason to stop one child trial early.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopCondition {
    /// Stop when the child settles a terminal victory or defeat.
    TerminalOutcome,
    /// Stop at the declared per-child decision bound.
    MaxDecisions,
    /// Stop when a declared budget bound is exhausted.
    BudgetExhausted,
}

impl StopCondition {
    /// Returns the stable lowercase label of this stop condition.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::TerminalOutcome => "terminal_outcome",
            Self::MaxDecisions => "max_decisions",
            Self::BudgetExhausted => "budget_exhausted",
        }
    }
}

/// One child policy and, when the strategy fixes it, its explicit first action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChildPolicy {
    /// Stable child label, unique within the experiment.
    pub child_label: String,
    /// Digest of the provider, model, prompt and tool settings the child may use.
    pub settings_digest: String,
    /// Explicit first action under [`ForkStrategy::AlternativeFirstAction`].
    pub first_action: Option<String>,
}

/// Declared per-child and total bounds for one experiment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BranchBudgets {
    /// Maximum decisions one child may take.
    pub max_decisions_per_child: u64,
    /// Maximum decisions across every child together.
    pub max_total_decisions: u64,
    /// Maximum wall-clock milliseconds across the experiment.
    pub max_total_duration_millis: u64,
    /// Maximum provider spend in micros, when a bound exists.
    pub max_provider_spend_micros: Option<u64>,
    /// Maximum concurrently running children.
    pub max_concurrency: u32,
}

/// A bounded same-start branch experiment declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchExperimentManifest {
    /// Version tag; must equal [`BRANCH_EXPERIMENT_VERSION`].
    pub version: String,
    /// Experiment identifier, kept outside exact game identity.
    pub experiment_id: String,
    /// Reference to the benchmark declaration this experiment belongs to.
    pub benchmark_ref: String,
    /// The one verified checkpoint every child restores.
    pub fork_point: ExactCheckpointReference,
    /// How each child's first action is chosen.
    pub strategy: ForkStrategy,
    /// Declared provider/context policy for every child.
    pub context_policy: String,
    /// Declared stop conditions; at least one.
    pub stop_conditions: Vec<StopCondition>,
    /// Declared budgets.
    pub budgets: BranchBudgets,
    /// Child policies; at least one and at most [`MAX_BRANCH_CHILDREN`].
    pub children: Vec<ChildPolicy>,
}

/// A keyed public summary of one experiment; it carries no exact digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchExperimentPublic {
    /// Public envelope version.
    pub version: &'static str,
    /// Keyed opaque handle of the verified fork point.
    pub handle: String,
    /// Declared fork strategy label.
    pub strategy: &'static str,
    /// Declared child count.
    pub child_count: usize,
}

impl BranchExperimentManifest {
    /// Validates every declared label, axis and bound.
    ///
    /// # Errors
    ///
    /// Returns the specific error for the first failing field.
    pub fn validate(&self) -> Result<(), BranchExperimentError> {
        if self.version != BRANCH_EXPERIMENT_VERSION {
            return Err(BranchExperimentError::UnsupportedVersion);
        }
        for label in [
            &self.experiment_id,
            &self.benchmark_ref,
            &self.context_policy,
        ] {
            if !label_ok(label) {
                return Err(BranchExperimentError::InvalidLabel);
            }
        }
        self.fork_point
            .validate()
            .map_err(|_| BranchExperimentError::InvalidForkPoint)?;
        if self.stop_conditions.is_empty() {
            return Err(BranchExperimentError::EmptyStopConditions);
        }
        self.validate_budgets()?;
        self.validate_children()
    }

    fn validate_budgets(&self) -> Result<(), BranchExperimentError> {
        let budgets = self.budgets;
        if budgets.max_decisions_per_child == 0
            || budgets.max_total_decisions < budgets.max_decisions_per_child
            || budgets.max_total_duration_millis == 0
            || budgets.max_concurrency == 0
            || budgets.max_concurrency > MAX_BRANCH_CONCURRENCY
            || budgets.max_provider_spend_micros == Some(0)
        {
            return Err(BranchExperimentError::InvalidBudget);
        }
        Ok(())
    }

    fn validate_children(&self) -> Result<(), BranchExperimentError> {
        if self.children.is_empty() {
            return Err(BranchExperimentError::EmptyChildren);
        }
        if self.children.len() > MAX_BRANCH_CHILDREN {
            return Err(BranchExperimentError::TooManyChildren);
        }
        let mut seen = BTreeSet::new();
        for child in &self.children {
            if !child_label_ok(&child.child_label) {
                return Err(BranchExperimentError::InvalidLabel);
            }
            if !seen.insert(child.child_label.as_str()) {
                return Err(BranchExperimentError::DuplicateChild);
            }
            if !digest_ok(&child.settings_digest) {
                return Err(BranchExperimentError::InvalidSettingsDigest);
            }
            match (self.strategy, &child.first_action) {
                (ForkStrategy::AlternativeFirstAction, Some(action)) if label_ok(action) => {}
                (ForkStrategy::AlternativeFirstAction, _) => {
                    return Err(BranchExperimentError::AlternativeFirstActionRequired);
                }
                (ForkStrategy::AlternatePolicy, Some(_)) => {
                    return Err(BranchExperimentError::UnexpectedFirstAction);
                }
                (ForkStrategy::AlternatePolicy, None) => {}
            }
        }
        Ok(())
    }

    /// Returns the declared child with this label.
    #[must_use]
    pub fn child(&self, child_label: &str) -> Option<&ChildPolicy> {
        self.children
            .iter()
            .find(|child| child.child_label == child_label)
    }

    /// Returns the number of planned trials, one per declared child.
    #[must_use]
    pub fn planned_count(&self) -> usize {
        self.children.len()
    }

    /// Returns the canonical SHA-256 digest used as the experiment revision identity.
    ///
    /// # Errors
    ///
    /// Returns the declaration rejection, or [`BranchExperimentError::TooLarge`].
    pub fn digest(&self) -> Result<String, BranchExperimentError> {
        self.validate()?;
        let mut payload = Vec::new();
        payload.extend_from_slice(b"AI-ASCENSION/BRANCH-EXPERIMENT/v1\0");
        for field in [
            self.version.as_str(),
            self.experiment_id.as_str(),
            self.benchmark_ref.as_str(),
            self.fork_point.exact_state_digest.as_str(),
            self.fork_point.exact_checkpoint_id.as_str(),
            self.fork_point.boundary_kind.as_str(),
            self.fork_point.boundary_phase.as_str(),
            self.fork_point.assurance.as_str(),
            self.strategy.label(),
            self.context_policy.as_str(),
        ] {
            payload.extend_from_slice(field.as_bytes());
            payload.push(0);
        }
        for stop in &self.stop_conditions {
            payload.extend_from_slice(stop.label().as_bytes());
            payload.push(0);
        }
        let budgets = self.budgets;
        for field in [
            budgets.max_decisions_per_child.to_string(),
            budgets.max_total_decisions.to_string(),
            budgets.max_total_duration_millis.to_string(),
            budgets
                .max_provider_spend_micros
                .map_or_else(|| "-".to_owned(), |value| value.to_string()),
            budgets.max_concurrency.to_string(),
        ] {
            payload.extend_from_slice(field.as_bytes());
            payload.push(0);
        }
        for child in &self.children {
            payload.extend_from_slice(child.child_label.as_bytes());
            payload.push(0);
            payload.extend_from_slice(child.settings_digest.as_bytes());
            payload.push(0);
            payload.extend_from_slice(child.first_action.as_deref().unwrap_or("-").as_bytes());
            payload.push(0);
        }
        if payload.len() > MAX_MANIFEST_BYTES {
            return Err(BranchExperimentError::TooLarge);
        }
        Ok(sha256_hex(&payload))
    }

    /// Derives a keyed public summary of the verified fork point and declared axes.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError`] when the declaration or projection key is unusable.
    pub fn public_projection(
        &self,
        key: &ProjectionKey,
    ) -> Result<BranchExperimentPublic, ProjectionError> {
        let revision = self.digest().map_err(|_| ProjectionError::InvalidInput)?;
        let occurrence = OccurrenceId::parse(&format!("branch-experiment-{revision}"))
            .map_err(|_| ProjectionError::InvalidInput)?;
        let handle = key.handle(
            &self.fork_point.exact_checkpoint_id,
            &self.fork_point.exact_state_digest,
            &occurrence,
        )?;
        Ok(BranchExperimentPublic {
            version: "ascension.branch-experiment-public.v1",
            handle,
            strategy: self.strategy.label(),
            child_count: self.children.len(),
        })
    }
}
