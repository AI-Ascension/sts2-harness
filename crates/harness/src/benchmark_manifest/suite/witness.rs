// SPDX-License-Identifier: MIT

//! Initial-witness audit: group each case's trials by their verified native start.
//!
//! Later divergence in actions, RNG consumption or outcome is expected and is not a defect. Only
//! two different *initial* witnesses for the same case are rejected, and a trial with no verified
//! witness is excluded from every exact-start group instead of being blended into one.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::error::ReportError;
use super::index::{outcome_index, planner_for};
use super::manifest::SuiteManifest;
use super::plan::PlannedSuiteTrial;
use super::results::TrialOutcome;

/// One planned trial inside an initial-witness group.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StartMember {
    /// Seed case the trial starts.
    pub case_id: String,
    /// Policy the trial evaluates.
    pub policy_id: String,
    /// Repetition of the (case, policy) pair.
    pub repetition: u32,
}

/// Every verified trial that started one case from the same native witness.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StartGroup {
    /// Seed case shared by every member.
    pub case_id: String,
    /// Verified exact-state witness shared by every member.
    pub witness: String,
    /// Verified members in plan order.
    pub members: Vec<StartMember>,
}

/// The initial-witness audit: verified groups plus trials that carry no verified witness.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WitnessAudit {
    /// One group per case with a verified witness.
    pub groups: Vec<StartGroup>,
    /// Planned trials with no verified witness, excluded from every exact-start group.
    pub unverified: Vec<StartMember>,
}

/// Groups each case's trials by their verified initial native witness.
///
/// # Errors
///
/// Returns [`ReportError::WitnessMismatch`] when one case carries two different verified
/// witnesses, plus the same rejections as
/// [`ensure_metric_coverage`](super::report::ensure_metric_coverage).
pub fn audit_initial_witness(
    manifest: &SuiteManifest,
    outcomes: &[TrialOutcome],
) -> Result<WitnessAudit, ReportError> {
    let planner = planner_for(manifest)?;
    let index = outcome_index(&planner, outcomes)?;
    let mut groups: BTreeMap<String, StartGroup> = BTreeMap::new();
    let mut unverified = Vec::new();
    for trial in &planner {
        let member = member_of(trial);
        let witness = index
            .get(trial.trial_key.as_str())
            .and_then(|outcome| outcome.initial_witness.as_ref());
        match witness {
            Some(witness) => insert_witness(&mut groups, &trial.case_id, witness.as_str(), member)?,
            None => unverified.push(member),
        }
    }
    Ok(WitnessAudit {
        groups: groups.into_values().collect(),
        unverified,
    })
}

fn member_of(trial: &PlannedSuiteTrial) -> StartMember {
    StartMember {
        case_id: trial.case_id.clone(),
        policy_id: trial.policy_id.clone(),
        repetition: trial.repetition,
    }
}

fn insert_witness(
    groups: &mut BTreeMap<String, StartGroup>,
    case_id: &str,
    witness: &str,
    member: StartMember,
) -> Result<(), ReportError> {
    match groups.get_mut(case_id) {
        Some(group) if group.witness == witness => {
            group.members.push(member);
            Ok(())
        }
        Some(_) => Err(ReportError::WitnessMismatch(case_id.to_owned())),
        None => {
            groups.insert(
                case_id.to_owned(),
                StartGroup {
                    case_id: case_id.to_owned(),
                    witness: witness.to_owned(),
                    members: vec![member],
                },
            );
            Ok(())
        }
    }
}
