// SPDX-License-Identifier: MIT

//! Same-start admission: every trial re-proves the shared verified fork point before it runs.
//!
//! A child is admitted only against one observed start that names the same exact checkpoint and
//! carries a verified restore. A prefix-only or capture-only observation is refused here, so an
//! unverified start can never be recorded as if it were the shared exact start.

use crate::{ExactAssurance, ExactCheckpointReference};

use super::declaration::BranchExperimentManifest;
use super::error::AdmissionError;
use super::plan::PlannedBranchTrial;

/// The recorded admission of one trial to the shared verified fork point.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartAdmission {
    /// Plan key of the admitted trial.
    pub trial_key: String,
    /// The observed start reference that was admitted.
    pub start: ExactCheckpointReference,
}

impl StartAdmission {
    /// Reports whether this admission carries a verified exact restore.
    #[must_use]
    pub fn is_exact_restore(&self) -> bool {
        is_verified_start(self.start.assurance)
    }
}

/// Reports whether an assurance level proves a verified exact restore of shared state.
#[must_use]
pub fn is_verified_start(assurance: ExactAssurance) -> bool {
    matches!(
        assurance,
        ExactAssurance::RestoreVerified | ExactAssurance::ContinuationCertified
    )
}

/// Admits one trial to the shared fork point, refusing anything but the same verified start.
///
/// # Errors
///
/// Returns [`AdmissionError::UnknownTrial`] when the trial is not part of the plan,
/// [`AdmissionError::InvalidReference`] when the observation is malformed,
/// [`AdmissionError::StartMismatch`] when it names a different exact checkpoint, and
/// [`AdmissionError::StartNotVerified`] when it carries no verified restore.
pub fn admit_start(
    manifest: &BranchExperimentManifest,
    trial: &PlannedBranchTrial,
    observed: &ExactCheckpointReference,
) -> Result<StartAdmission, AdmissionError> {
    if manifest.child(&trial.child_label).is_none() {
        return Err(AdmissionError::UnknownTrial(trial.trial_key.clone()));
    }
    observed
        .validate()
        .map_err(|_| AdmissionError::InvalidReference)?;
    let fork_point = &manifest.fork_point;
    if observed.exact_state_digest != fork_point.exact_state_digest
        || observed.exact_checkpoint_id != fork_point.exact_checkpoint_id
    {
        return Err(AdmissionError::StartMismatch);
    }
    if !is_verified_start(observed.assurance) {
        return Err(AdmissionError::StartNotVerified);
    }
    Ok(StartAdmission {
        trial_key: trial.trial_key.clone(),
        start: observed.clone(),
    })
}
