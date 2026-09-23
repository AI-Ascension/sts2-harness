// SPDX-License-Identifier: MIT

//! Honest paired comparisons between two policies over the same seed cases.
//!
//! Only repetitions that settled a decided outcome in both policies are paired. The missing pairs
//! are reported beside the sample size, and the win-rate delta is `None` when nothing paired
//! rather than a fabricated zero.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::error::ReportError;
use super::index::{outcome_index, planner_for, policy_ids};
use super::manifest::SuiteManifest;
use super::plan::trial_key;
use super::results::{TrialOutcome, TrialResult};

/// One honest paired comparison of two policies over the same cases.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PairedComparison {
    /// Seed case compared.
    pub case_id: String,
    /// First policy.
    pub policy_a: String,
    /// Second policy.
    pub policy_b: String,
    /// Repetitions planned for the pair.
    pub planned_pairs: u64,
    /// Repetitions that settled a decided outcome in both policies.
    pub paired: u64,
    /// Repetitions where policy A settled no decided outcome.
    pub a_missing: u64,
    /// Repetitions where policy B settled no decided outcome.
    pub b_missing: u64,
    /// Repetitions where neither policy settled a decided outcome.
    pub both_missing: u64,
    /// Paired repetitions won by policy A.
    pub a_wins: u64,
    /// Paired repetitions won by policy B.
    pub b_wins: u64,
}

impl PairedComparison {
    /// Returns the A-minus-B win-rate delta in parts-per-million, or `None` when nothing paired.
    #[must_use]
    pub fn delta_win_rate_ppm(&self) -> Option<i64> {
        let a = i64::try_from(self.a_wins).ok()?;
        let b = i64::try_from(self.b_wins).ok()?;
        let paired = i64::try_from(self.paired).ok()?;
        (paired > 0).then(|| (a - b) * 1_000_000 / paired)
    }
}

/// Compares two policies case by case over the same repetitions.
///
/// # Errors
///
/// Returns the manifest rejection, an unknown policy, or a repeated policy selection, plus the
/// same outcome rejections as [`ensure_metric_coverage`](super::report::ensure_metric_coverage).
pub fn compare_paired_policies(
    manifest: &SuiteManifest,
    outcomes: &[TrialOutcome],
    policy_a: &str,
    policy_b: &str,
) -> Result<Vec<PairedComparison>, ReportError> {
    if policy_a == policy_b {
        return Err(ReportError::RepeatedPolicy(policy_a.to_owned()));
    }
    let policies = policy_ids(manifest);
    for policy in [policy_a, policy_b] {
        if !policies.contains(policy) {
            return Err(ReportError::UnknownPolicy(policy.to_owned()));
        }
    }
    let planner = planner_for(manifest)?;
    let index = outcome_index(&planner, outcomes)?;
    let revision = manifest.digest()?;
    let mut comparisons = Vec::new();
    for case in &manifest.corpus.cases {
        comparisons.push(compare_case(
            manifest.repetitions,
            &revision,
            &index,
            &case.case_id,
            policy_a,
            policy_b,
        ));
    }
    Ok(comparisons)
}

fn compare_case(
    repetitions: u32,
    revision: &str,
    index: &BTreeMap<&str, &TrialOutcome>,
    case_id: &str,
    policy_a: &str,
    policy_b: &str,
) -> PairedComparison {
    let mut comparison = PairedComparison {
        case_id: case_id.to_owned(),
        policy_a: policy_a.to_owned(),
        policy_b: policy_b.to_owned(),
        planned_pairs: u64::from(repetitions),
        paired: 0,
        a_missing: 0,
        b_missing: 0,
        both_missing: 0,
        a_wins: 0,
        b_wins: 0,
    };
    for repetition in 0..repetitions {
        let a = decided_result(index, &trial_key(revision, case_id, policy_a, repetition));
        let b = decided_result(index, &trial_key(revision, case_id, policy_b, repetition));
        if a.is_none() {
            comparison.a_missing += 1;
        }
        if b.is_none() {
            comparison.b_missing += 1;
        }
        comparison.paired += u64::from(a.is_some() && b.is_some());
        if let (Some(a_result), Some(b_result)) = (a, b) {
            score_pair(&mut comparison, a_result, b_result);
        } else if a.is_none() && b.is_none() {
            comparison.both_missing += 1;
        }
    }
    comparison
}

fn score_pair(comparison: &mut PairedComparison, a_result: TrialResult, b_result: TrialResult) {
    match (a_result, b_result) {
        (TrialResult::Victory, TrialResult::Defeat) => comparison.a_wins += 1,
        (TrialResult::Defeat, TrialResult::Victory) => comparison.b_wins += 1,
        (TrialResult::Victory, TrialResult::Victory)
        | (TrialResult::Defeat, TrialResult::Defeat) => {}
    }
}

fn decided_result(index: &BTreeMap<&str, &TrialOutcome>, key: &str) -> Option<TrialResult> {
    index.get(key).and_then(|outcome| outcome.result)
}
