// SPDX-License-Identifier: MIT

//! Same-start policy experiments and ancestry-based dataset splits.
//!
//! Every branch restores the same verified start through an opaque handle, then writes only to its
//! own artifact scope, so sibling branches cannot contaminate each other. Policy, model, prompt,
//! and tool settings are recorded as branch metadata and never enter exact game identity. Dataset
//! splits group by ancestry root, so duplicate starting states and their descendants stay in one
//! split.

use std::collections::BTreeMap;

use crate::execution::ExactCheckpointReference;

use super::MAX_TRANSITION_LABEL_BYTES;
use super::lineage::{LineageError, OccurrenceGraph, OccurrenceId};

/// Maximum branches in one experiment.
pub const MAX_BRANCHES: usize = 64;

/// Rejection reasons for experiment branches.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExperimentError {
    /// A label is empty, too long, or contains a NUL separator.
    InvalidLabel,
    /// The start checkpoint reference is invalid.
    InvalidStart,
    /// The branch identifier or handle is not opaque or repeats another branch.
    InvalidHandle,
    /// The branch write scope repeats another branch.
    DuplicateScope,
    /// The experiment already holds [`MAX_BRANCHES`].
    Capacity,
    /// The experiment has no branch to evaluate.
    NoBranches,
}

/// Policy and provider settings kept outside exact game identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchPolicy {
    /// Policy artifact or model identifier.
    pub label: String,
    /// Digest of provider, prompt, and tool settings used by this branch.
    pub settings_digest: String,
}

/// One independently writing branch restored from the shared start.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExperimentBranch {
    /// Opaque handle exposed to the policy.
    pub branch_id: String,
    /// Root occurrence recorded for this branch.
    pub occurrence: OccurrenceId,
    /// Policy settings, never part of exact game identity.
    pub policy: BranchPolicy,
    /// Independent artifact write scope.
    pub write_scope: String,
}

/// A same-start experiment over one verified checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Experiment {
    /// Experiment identifier.
    pub experiment_id: String,
    /// The verified starting checkpoint every branch restores.
    pub start: ExactCheckpointReference,
    branches: Vec<ExperimentBranch>,
}

impl Experiment {
    /// Creates an experiment over a validated start reference.
    pub fn new(
        experiment_id: &str,
        start: ExactCheckpointReference,
    ) -> Result<Self, ExperimentError> {
        require_label(experiment_id)?;
        start
            .validate()
            .map_err(|_| ExperimentError::InvalidStart)?;
        Ok(Self {
            experiment_id: experiment_id.to_owned(),
            start,
            branches: Vec::new(),
        })
    }

    /// Adds a branch with an opaque handle, its own occurrence, and its own write scope.
    pub fn fork(
        &mut self,
        branch_id: &str,
        occurrence: OccurrenceId,
        policy: BranchPolicy,
        write_scope: &str,
    ) -> Result<&ExperimentBranch, ExperimentError> {
        require_label(branch_id)?;
        require_label(write_scope)?;
        require_label(&policy.label)?;
        require_digest(&policy.settings_digest)?;
        if self.branches.len() >= MAX_BRANCHES {
            return Err(ExperimentError::Capacity);
        }
        if handle_exposes_identity(branch_id, &self.start) {
            return Err(ExperimentError::InvalidHandle);
        }
        if self
            .branches
            .iter()
            .any(|branch| branch.branch_id == branch_id || branch.write_scope == write_scope)
        {
            return Err(
                if self
                    .branches
                    .iter()
                    .any(|branch| branch.write_scope == write_scope)
                {
                    ExperimentError::DuplicateScope
                } else {
                    ExperimentError::InvalidHandle
                },
            );
        }
        self.branches.push(ExperimentBranch {
            branch_id: branch_id.to_owned(),
            occurrence,
            policy,
            write_scope: write_scope.to_owned(),
        });
        self.branches.last().ok_or(ExperimentError::Capacity)
    }

    /// Returns the recorded branches.
    #[must_use]
    pub fn branches(&self) -> &[ExperimentBranch] {
        &self.branches
    }

    /// Reports whether at least one branch was forked.
    #[must_use]
    pub fn has_branches(&self) -> bool {
        !self.branches.is_empty()
    }
}

/// A deterministic ancestry-respecting dataset split.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AncestrySplit {
    /// Roots and descendants assigned to training.
    pub train: Vec<OccurrenceId>,
    /// Roots and descendants assigned to validation.
    pub validation: Vec<OccurrenceId>,
    /// Roots and descendants assigned to test.
    pub test: Vec<OccurrenceId>,
}

impl AncestrySplit {
    /// Returns occurrences present in more than one split; must always be empty.
    #[must_use]
    pub fn overlap(&self) -> Vec<OccurrenceId> {
        let mut counts: BTreeMap<&OccurrenceId, u8> = BTreeMap::new();
        for group in [&self.train, &self.validation, &self.test] {
            for occurrence in group {
                *counts.entry(occurrence).or_insert(0) += 1;
            }
        }
        counts
            .into_iter()
            .filter(|(_, count)| *count > 1)
            .map(|(occurrence, _)| occurrence.clone())
            .collect()
    }

    /// Returns the number of occurrences in each split.
    #[must_use]
    pub fn sizes(&self) -> (usize, usize, usize) {
        (self.train.len(), self.validation.len(), self.test.len())
    }

    /// Returns the total number of assigned occurrences.
    #[must_use]
    pub fn len(&self) -> usize {
        self.train.len() + self.validation.len() + self.test.len()
    }

    /// Reports whether no occurrence was assigned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Splits occurrences into families keyed by ancestry root and duplicate starting state.
///
/// Roots that recorded the same exact state identity are merged into one family, so duplicate
/// starts and their descendants always land in the same split. Whole families are assigned in
/// stable digest order.
pub fn split_by_ancestry(
    graph: &OccurrenceGraph,
    train_percent: u8,
    validation_percent: u8,
) -> Result<AncestrySplit, LineageError> {
    if u16::from(train_percent) + u16::from(validation_percent) > 100 {
        return Err(LineageError::InvalidRecord);
    }
    let groups = graph.group_by_root()?;
    let mut families: BTreeMap<String, Vec<OccurrenceId>> = BTreeMap::new();
    for (root, members) in &groups {
        let key = graph.state_digest(root)?.as_str().to_owned();
        families
            .entry(key)
            .or_default()
            .extend(members.iter().cloned());
    }
    let ordered: Vec<Vec<OccurrenceId>> = families.into_values().collect();
    let total = ordered.len();
    let train_families = (total * usize::from(train_percent)).div_ceil(100);
    let validation_families = (total * usize::from(validation_percent)).div_ceil(100);
    let mut split = AncestrySplit::default();
    for (index, members) in ordered.into_iter().enumerate() {
        if index < train_families {
            split.train.extend(members);
        } else if index < train_families + validation_families {
            split.validation.extend(members);
        } else {
            split.test.extend(members);
        }
    }
    Ok(split)
}

fn require_label(value: &str) -> Result<(), ExperimentError> {
    if value.is_empty() || value.len() > MAX_TRANSITION_LABEL_BYTES || value.contains('\0') {
        return Err(ExperimentError::InvalidLabel);
    }
    Ok(())
}

fn require_digest(value: &str) -> Result<(), ExperimentError> {
    let hex = value.strip_prefix("sha256:").unwrap_or("");
    let lowercase = hex
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if hex.len() != 64 || !lowercase {
        return Err(ExperimentError::InvalidLabel);
    }
    Ok(())
}

fn handle_exposes_identity(handle: &str, start: &ExactCheckpointReference) -> bool {
    [
        start.exact_state_digest.as_str(),
        start.exact_checkpoint_id.as_str(),
    ]
    .iter()
    .any(|digest| {
        let hex = digest.rsplit(':').next().unwrap_or(digest);
        handle.contains(hex)
    })
}
