// SPDX-License-Identifier: MIT

//! The sanitized aggregate report: explicit denominators and metric availability.
//!
//! The aggregate is machine-readable and carries case/policy identifiers, explicit denominators
//! and metric availability counts, but no seeds, digests or witness values. Unmeasured quantities
//! are reported as unavailable rather than zero, and infrastructure failures never count as
//! defeats.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::error::ReportError;
use super::index::{count, outcome_index, planner_for};
use super::manifest::SuiteManifest;
use super::plan::trial_key;
use super::results::{Metric, TrialOutcome, TrialResult, TrialStatus};

/// Explicit per-cell denominators for one (case, policy) pair.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CellReport {
    /// Seed case of this cell.
    pub case_id: String,
    /// Policy of this cell.
    pub policy_id: String,
    /// Planned repetitions of this pair.
    pub planned: u64,
    /// Repetitions that recorded any outcome.
    pub recorded: u64,
    /// Recorded victories.
    pub victories: u64,
    /// Recorded defeats.
    pub defeats: u64,
    /// Trials stopped by a declared budget bound.
    pub censored: u64,
    /// Cancelled trials.
    pub cancelled: u64,
    /// Trials that failed in infrastructure.
    pub infrastructure_failures: u64,
    /// Trials whose outcome stayed unknown.
    pub unknown: u64,
    /// Planned repetitions with no recorded outcome at all.
    pub missing: u64,
}

impl CellReport {
    fn empty(case_id: &str, policy_id: &str, planned: u64) -> Self {
        Self {
            case_id: case_id.to_owned(),
            policy_id: policy_id.to_owned(),
            planned,
            recorded: 0,
            victories: 0,
            defeats: 0,
            censored: 0,
            cancelled: 0,
            infrastructure_failures: 0,
            unknown: 0,
            missing: planned,
        }
    }

    fn record(&mut self, outcome: &TrialOutcome) {
        self.recorded += 1;
        match (outcome.status, outcome.result) {
            (TrialStatus::Completed, Some(TrialResult::Victory)) => self.victories += 1,
            (TrialStatus::Completed, Some(TrialResult::Defeat)) => self.defeats += 1,
            (TrialStatus::BudgetCensored, _) => self.censored += 1,
            (TrialStatus::Cancelled, _) => self.cancelled += 1,
            (TrialStatus::InfrastructureFailure, _) => self.infrastructure_failures += 1,
            (TrialStatus::UnknownOutcome, _) | (TrialStatus::Completed, None) => self.unknown += 1,
        }
    }

    /// Returns the number of repetitions that settled a victory or a defeat.
    #[must_use]
    pub fn decided(&self) -> u64 {
        self.victories + self.defeats
    }

    /// Returns the decided share in parts-per-million, or `None` when nothing was recorded.
    #[must_use]
    pub fn decision_rate_ppm(&self) -> Option<u64> {
        (self.recorded > 0).then(|| self.decided() * 1_000_000 / self.recorded)
    }
}

/// How many recorded trials measured one predeclared metric, and how many left it unavailable.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MetricAvailability {
    /// Stable metric label.
    pub metric: String,
    /// Trials that measured the metric.
    pub measured: u64,
    /// Trials that declared the metric unavailable.
    pub unavailable: u64,
}

/// Completeness and metric availability over a whole suite.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MetricCoverage {
    /// Planned logical trials.
    pub planned_trials: u64,
    /// Trials with a recorded outcome.
    pub recorded_trials: u64,
    /// Planned trials with no recorded outcome.
    pub missing_trials: u64,
    /// Per-metric availability counts.
    pub metrics: Vec<MetricAvailability>,
}

/// The sanitized aggregate report for one suite revision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SuiteReport {
    /// Suite revision identity the report belongs to.
    pub suite_revision: String,
    /// Evaluator revision the report belongs to.
    pub evaluator_revision: String,
    /// Planned logical trials.
    pub planned_trials: u64,
    /// Trials with a recorded outcome.
    pub recorded_trials: u64,
    /// Per-metric availability over recorded trials.
    pub metrics: Vec<MetricAvailability>,
    /// Per-(case, policy) cells in plan order.
    pub cells: Vec<CellReport>,
}

impl SuiteReport {
    /// Returns the cell for one (case, policy) pair.
    #[must_use]
    pub fn cell(&self, case_id: &str, policy_id: &str) -> Option<&CellReport> {
        self.cells
            .iter()
            .find(|cell| cell.case_id == case_id && cell.policy_id == policy_id)
    }

    /// Encodes the sanitized report as pretty JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ReportError::NotEncodable`] when the report cannot be encoded.
    pub fn to_json_pretty(&self) -> Result<String, ReportError> {
        serde_json::to_string_pretty(self).map_err(|_| ReportError::NotEncodable)
    }
}

/// Verifies that every planned trial has exactly one valid outcome and every predeclared metric
/// is a known metric, then counts metric availability.
///
/// # Errors
///
/// Returns the manifest rejection, a malformed outcome, an unknown metric label, an off-plan
/// outcome key, or a duplicate outcome key.
pub fn ensure_metric_coverage(
    manifest: &SuiteManifest,
    outcomes: &[TrialOutcome],
) -> Result<MetricCoverage, ReportError> {
    let metrics = declared_metrics(manifest)?;
    let planner = planner_for(manifest)?;
    let index = outcome_index(&planner, outcomes)?;
    let planned_trials = count(planner.len());
    let recorded_trials = count(index.len());
    let mut availability = availability_slots(&metrics);
    for outcome in index.values() {
        for (slot, metric) in availability.iter_mut().zip(&metrics) {
            if metric.measured_value(outcome).is_some() {
                slot.measured += 1;
            } else {
                slot.unavailable += 1;
            }
        }
    }
    Ok(MetricCoverage {
        planned_trials,
        recorded_trials,
        missing_trials: planned_trials.saturating_sub(recorded_trials),
        metrics: availability,
    })
}

/// Builds the sanitized aggregate report for one suite revision.
///
/// # Errors
///
/// Returns the same rejections as [`ensure_metric_coverage`].
pub fn aggregate(
    manifest: &SuiteManifest,
    outcomes: &[TrialOutcome],
) -> Result<SuiteReport, ReportError> {
    let coverage = ensure_metric_coverage(manifest, outcomes)?;
    let planner = planner_for(manifest)?;
    let index = outcome_index(&planner, outcomes)?;
    let revision = manifest.digest()?;
    let mut cells = Vec::new();
    for case in &manifest.corpus.cases {
        for policy in &manifest.policies {
            cells.push(cell_for(
                manifest.repetitions,
                &revision,
                &index,
                &case.case_id,
                &policy.policy_id,
            ));
        }
    }
    Ok(SuiteReport {
        suite_revision: revision,
        evaluator_revision: manifest.evaluator_revision.clone(),
        planned_trials: coverage.planned_trials,
        recorded_trials: coverage.recorded_trials,
        metrics: coverage.metrics,
        cells,
    })
}

fn availability_slots(metrics: &[Metric]) -> Vec<MetricAvailability> {
    metrics
        .iter()
        .map(|metric| MetricAvailability {
            metric: metric.label().to_owned(),
            measured: 0,
            unavailable: 0,
        })
        .collect()
}

fn declared_metrics(manifest: &SuiteManifest) -> Result<Vec<Metric>, ReportError> {
    manifest
        .metrics
        .iter()
        .map(|label| {
            Metric::from_label(label).ok_or_else(|| ReportError::UnknownMetric(label.clone()))
        })
        .collect()
}

fn cell_for(
    repetitions: u32,
    revision: &str,
    index: &BTreeMap<&str, &TrialOutcome>,
    case_id: &str,
    policy_id: &str,
) -> CellReport {
    let mut cell = CellReport::empty(case_id, policy_id, u64::from(repetitions));
    for repetition in 0..repetitions {
        let key = trial_key(revision, case_id, policy_id, repetition);
        if let Some(outcome) = index.get(key.as_str()) {
            cell.record(outcome);
        }
    }
    cell.missing = cell.planned.saturating_sub(cell.recorded);
    cell
}
