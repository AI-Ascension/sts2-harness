// SPDX-License-Identifier: MIT

//! Machine-readable cold-start evidence for the verifier and the suite scheduler.

use serde::Serialize;

use super::baseline::{BaselineMismatch, PristineBaseline};
use super::lifecycle::TrialLifecycle;
use super::stage::ColdLaunchStage;

/// Machine-readable evidence for one cold-launch trial.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ColdStartEvidence {
    /// Trial the evidence belongs to.
    pub trial_key: String,
    /// Stage the trial reached.
    pub stage: ColdLaunchStage,
    /// Digest of the baseline the trial was admitted against.
    pub baseline_digest: String,
    /// Attested instance generation, when a process was born.
    pub instance_generation: Option<u64>,
    /// Attested birth token, when a process was born.
    pub birth_token: Option<String>,
    /// Leased destination, when one is still held.
    pub destination_id: Option<String>,
    /// Whether cleanup failed, independently of the gameplay outcome.
    pub cleanup_failed: bool,
    /// Reasons the trial's baseline differs from a reference, in stable order.
    pub mismatches: Vec<BaselineMismatch>,
}

/// Builds cold-start evidence for one trial against a reference baseline.
#[must_use]
pub fn evidence_of(trial: &TrialLifecycle, reference: &PristineBaseline) -> ColdStartEvidence {
    ColdStartEvidence {
        trial_key: trial.trial_key().to_owned(),
        stage: trial.stage(),
        baseline_digest: trial.baseline().baseline_digest.clone(),
        instance_generation: trial.birth().map(|birth| birth.instance_generation),
        birth_token: trial.birth().map(|birth| birth.birth_token.clone()),
        destination_id: trial
            .destination()
            .map(|destination| destination.destination_id.clone()),
        cleanup_failed: trial.cleanup_failed(),
        mismatches: trial.baseline().compare(reference),
    }
}
