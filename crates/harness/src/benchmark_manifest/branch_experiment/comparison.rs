// SPDX-License-Identifier: MIT

//! Honest aligned comparison of two child branches, plus the sanitized experiment report.
//!
//! Comparison is a linear aligned scan over logical actions, never endpoint equality, so an
//! identical final state does not erase an earlier divergence. An intentionally different first
//! action is policy divergence; a genesis mismatch or a failed start is a restore failure and is
//! never reported as a policy result. Only a verified start may enter exact-restore statistics.

use std::collections::{BTreeMap, BTreeSet};

use crate::{ProjectionKey, TraceOutcome, compare_traces};

use super::declaration::BranchExperimentManifest;
use super::error::{BranchExperimentError, ComparisonError};
use super::outcome::{BranchOutcome, BranchStatus};
use super::plan::{PlannedBranchTrial, plan};
use super::report::{BranchComparison, BranchDivergence, BranchExperimentReport};

/// Compares two declared children by aligned logical action.
///
/// # Errors
///
/// Returns [`ComparisonError`] for an unknown or repeated child, a rejected declaration, a
/// malformed outcome, an off-plan outcome key, or a duplicate outcome key.
pub fn compare_branches(
    manifest: &BranchExperimentManifest,
    outcomes: &[BranchOutcome],
    child_a: &str,
    child_b: &str,
) -> Result<BranchComparison, ComparisonError> {
    if child_a == child_b {
        return Err(ComparisonError::RepeatedChild(child_a.to_owned()));
    }
    let policy_a = manifest
        .child(child_a)
        .ok_or_else(|| ComparisonError::UnknownChild(child_a.to_owned()))?;
    let policy_b = manifest
        .child(child_b)
        .ok_or_else(|| ComparisonError::UnknownChild(child_b.to_owned()))?;
    let planned = plan(manifest).map_err(ComparisonError::Declaration)?;
    let index = outcome_index(&planned, outcomes)?;
    let (Some(outcome_a), Some(outcome_b)) = (
        lookup(&planned, &index, child_a)?,
        lookup(&planned, &index, child_b)?,
    ) else {
        return Ok(comparison(
            child_a,
            child_b,
            BranchDivergence::MissingCapture,
            false,
            None,
            None,
            0,
        ));
    };
    let exact_restore = outcome_a.exact_restore_eligible() && outcome_b.exact_restore_eligible();
    if matches!(outcome_a.status, BranchStatus::RestoreFailed)
        || matches!(outcome_b.status, BranchStatus::RestoreFailed)
    {
        return Ok(comparison(
            child_a,
            child_b,
            BranchDivergence::RestoreFailure,
            exact_restore,
            None,
            None,
            0,
        ));
    }
    let (Some(trace_a), Some(trace_b)) = (&outcome_a.trace, &outcome_b.trace) else {
        return Ok(comparison(
            child_a,
            child_b,
            BranchDivergence::MissingCapture,
            exact_restore,
            None,
            None,
            0,
        ));
    };
    let compared = compare_traces(trace_a, trace_b)
        .map_err(|_| ComparisonError::InvalidOutcome(BranchExperimentError::InvalidOutcome))?;
    let declared_divergence = policy_a.settings_digest != policy_b.settings_digest
        || policy_a.first_action != policy_b.first_action;
    Ok(comparison(
        child_a,
        child_b,
        classify(compared.outcome, declared_divergence),
        exact_restore,
        compared.first_unequal_ordinal,
        compared.last_equal_ordinal,
        u64::try_from(compared.compared_records).unwrap_or(u64::MAX),
    ))
}

/// Builds the sanitized aggregate report with a keyed experiment handle.
///
/// # Errors
///
/// Returns the same rejections as [`compare_branches`], plus [`ComparisonError::InvalidKey`] when
/// the projection key is unusable.
pub fn aggregate(
    manifest: &BranchExperimentManifest,
    outcomes: &[BranchOutcome],
    key: &ProjectionKey,
) -> Result<BranchExperimentReport, ComparisonError> {
    let planned = plan(manifest).map_err(ComparisonError::Declaration)?;
    let index = outcome_index(&planned, outcomes)?;
    let mut settled = 0_u64;
    let mut censored = 0_u64;
    let mut cancelled = 0_u64;
    let mut unknown = 0_u64;
    let mut restore_failures = 0_u64;
    let mut exact_restore_settled = 0_u64;
    for outcome in index.values() {
        match outcome.status {
            BranchStatus::Completed => settled += 1,
            BranchStatus::BudgetCensored => censored += 1,
            BranchStatus::Cancelled => cancelled += 1,
            BranchStatus::InfrastructureFailure | BranchStatus::UnknownOutcome => unknown += 1,
            BranchStatus::RestoreFailed => restore_failures += 1,
        }
        exact_restore_settled += u64::from(outcome.exact_restore_eligible());
    }
    let mut comparisons = Vec::new();
    for (position, first) in manifest.children.iter().enumerate() {
        for second in manifest.children.iter().skip(position + 1) {
            comparisons.push(compare_branches(
                manifest,
                outcomes,
                &first.child_label,
                &second.child_label,
            )?);
        }
    }
    let public = manifest
        .public_projection(key)
        .map_err(|_| ComparisonError::InvalidKey)?;
    Ok(BranchExperimentReport {
        version: "ascension.branch-experiment-report.v1",
        experiment_ref: public.handle,
        strategy: manifest.strategy.label(),
        planned: u64::try_from(planned.len()).unwrap_or(u64::MAX),
        settled,
        censored,
        cancelled,
        unknown,
        restore_failures,
        exact_restore_settled,
        comparisons,
    })
}

fn classify(outcome: TraceOutcome, declared_divergence: bool) -> BranchDivergence {
    match outcome {
        TraceOutcome::IdenticalOverRecordedRange => BranchDivergence::IdenticalOverRecordedRange,
        TraceOutcome::DifferentAction if declared_divergence => BranchDivergence::PolicyDivergence,
        TraceOutcome::DifferentAction => BranchDivergence::DifferentAction,
        TraceOutcome::DifferentExternalInput => BranchDivergence::DifferentExternalInput,
        TraceOutcome::StateDivergence => BranchDivergence::StateDivergence,
        TraceOutcome::MissingCapture => BranchDivergence::MissingCapture,
        TraceOutcome::RestoreMismatch => BranchDivergence::RestoreFailure,
        TraceOutcome::IncompatibleProfile => BranchDivergence::IncompatibleTrace,
        TraceOutcome::UnalignedTrace => BranchDivergence::UnalignedTrace,
        TraceOutcome::InsufficientCoverage => BranchDivergence::InsufficientCoverage,
    }
}

#[allow(clippy::too_many_arguments)]
fn comparison(
    child_a: &str,
    child_b: &str,
    divergence: BranchDivergence,
    exact_restore: bool,
    first_divergence_ordinal: Option<u64>,
    last_equal_ordinal: Option<u64>,
    compared_actions: u64,
) -> BranchComparison {
    BranchComparison {
        child_a: child_a.to_owned(),
        child_b: child_b.to_owned(),
        divergence,
        exact_restore,
        first_divergence_ordinal,
        last_equal_ordinal,
        compared_actions,
    }
}

fn outcome_index<'a>(
    planned: &[PlannedBranchTrial],
    outcomes: &'a [BranchOutcome],
) -> Result<BTreeMap<&'a str, &'a BranchOutcome>, ComparisonError> {
    let known: BTreeSet<&str> = planned
        .iter()
        .map(|trial| trial.trial_key.as_str())
        .collect();
    let mut index = BTreeMap::new();
    for outcome in outcomes {
        outcome
            .validate()
            .map_err(ComparisonError::InvalidOutcome)?;
        let key = outcome.trial_key.as_str();
        if !known.contains(key) {
            return Err(ComparisonError::UnplannedOutcome(key.to_owned()));
        }
        if index.insert(key, outcome).is_some() {
            return Err(ComparisonError::DuplicateOutcome(key.to_owned()));
        }
    }
    Ok(index)
}

fn lookup<'a>(
    planned: &[PlannedBranchTrial],
    index: &BTreeMap<&'a str, &'a BranchOutcome>,
    child: &str,
) -> Result<Option<&'a BranchOutcome>, ComparisonError> {
    let trial = planned
        .iter()
        .find(|trial| trial.child_label == child)
        .ok_or_else(|| ComparisonError::UnknownChild(child.to_owned()))?;
    Ok(index.get(trial.trial_key.as_str()).copied())
}
