// SPDX-License-Identifier: MIT

//! Stable trial keys and the deterministic case-major plan of a suite.

use super::manifest::{SuiteManifest, SuiteManifestError};
use super::results::MAX_TRIAL_KEY_BYTES;

/// Separator between the parts of a stable trial key.
pub const TRIAL_KEY_SEPARATOR: char = '/';
/// Prefix of the fresh per-trial provider/context namespace.
pub const CONTEXT_NAMESPACE_PREFIX: &str = "suite-trial:";

/// One stable logical trial derived from a suite manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedSuiteTrial {
    /// Stable suite/case/policy/repetition key.
    pub trial_key: String,
    /// Declared seed case this trial starts.
    pub case_id: String,
    /// Declared policy configuration this trial evaluates.
    pub policy_id: String,
    /// Zero-based repetition of the (case, policy) pair.
    pub repetition: u32,
    /// Fresh provider/context namespace reserved for this trial alone.
    pub context_namespace: String,
}

/// Builds a stable trial key from the suite revision and the case/policy/repetition axes.
#[must_use]
pub fn trial_key(suite_revision: &str, case_id: &str, policy_id: &str, repetition: u32) -> String {
    format!(
        "{suite_revision}{TRIAL_KEY_SEPARATOR}{case_id}{TRIAL_KEY_SEPARATOR}{policy_id}\
{TRIAL_KEY_SEPARATOR}{repetition}"
    )
}

/// Plans every logical trial in a deterministic case-major, policy-major order.
///
/// # Errors
///
/// Returns the manifest rejection for an invalid or unencodable declaration.
pub fn plan(manifest: &SuiteManifest) -> Result<Vec<PlannedSuiteTrial>, SuiteManifestError> {
    let revision = manifest.digest()?;
    let mut planned = Vec::new();
    for case in &manifest.corpus.cases {
        for policy in &manifest.policies {
            for repetition in 0..manifest.repetitions {
                let key = trial_key(&revision, &case.case_id, &policy.policy_id, repetition);
                debug_assert!(
                    key.len() <= MAX_TRIAL_KEY_BYTES,
                    "a validated manifest derives a settleable trial key"
                );
                planned.push(PlannedSuiteTrial {
                    context_namespace: format!("{CONTEXT_NAMESPACE_PREFIX}{key}"),
                    trial_key: key,
                    case_id: case.case_id.clone(),
                    policy_id: policy.policy_id.clone(),
                    repetition,
                });
            }
        }
    }
    Ok(planned)
}
