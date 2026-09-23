// SPDX-License-Identifier: MIT

//! Shared plan/outcome indexing used by every report surface.

use std::collections::{BTreeMap, BTreeSet};

use super::error::ReportError;
use super::manifest::SuiteManifest;
use super::plan::{PlannedSuiteTrial, plan};
use super::results::TrialOutcome;

/// Plans the suite, mapping the manifest rejection into a report rejection.
pub(super) fn planner_for(manifest: &SuiteManifest) -> Result<Vec<PlannedSuiteTrial>, ReportError> {
    plan(manifest).map_err(ReportError::Manifest)
}

/// Validates every outcome and indexes it by trial key, rejecting off-plan or duplicate keys.
pub(super) fn outcome_index<'a>(
    planner: &[PlannedSuiteTrial],
    outcomes: &'a [TrialOutcome],
) -> Result<BTreeMap<&'a str, &'a TrialOutcome>, ReportError> {
    let known: BTreeSet<&str> = planner
        .iter()
        .map(|trial| trial.trial_key.as_str())
        .collect();
    let mut index = BTreeMap::new();
    for outcome in outcomes {
        outcome.validate()?;
        let key = outcome.trial_key.as_str();
        if !known.contains(key) {
            return Err(ReportError::UnplannedOutcome(outcome.trial_key.clone()));
        }
        if index.insert(key, outcome).is_some() {
            return Err(ReportError::DuplicateOutcome(outcome.trial_key.clone()));
        }
    }
    Ok(index)
}

/// Returns the declared policy identifiers.
pub(super) fn policy_ids(manifest: &SuiteManifest) -> BTreeSet<&str> {
    manifest
        .policies
        .iter()
        .map(|policy| policy.policy_id.as_str())
        .collect()
}

/// Lossily widens a bounded count; only an impossible `usize` saturates.
pub(super) fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
