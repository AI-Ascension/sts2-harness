// SPDX-License-Identifier: MIT

//! Stable planning of one logical trial per declared child.
//!
//! Planning is effect-free and deterministic: it derives a stable trial key from the declaration
//! revision and the child label, and reserves one fresh provider/context namespace per trial so two
//! children that share a policy still cannot share context or provider state.

use super::declaration::BranchExperimentManifest;
use super::error::BranchExperimentError;
use super::label::{CONTEXT_NAMESPACE_PREFIX, MAX_BRANCH_LABEL_BYTES, TRIAL_KEY_SEPARATOR};

/// One stable logical trial derived from an experiment declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedBranchTrial {
    /// Stable experiment/child key.
    pub trial_key: String,
    /// Declared child this trial evaluates.
    pub child_label: String,
    /// Explicit first action, when the strategy fixes one.
    pub first_action: Option<String>,
    /// Fresh provider/context namespace reserved for this trial alone.
    pub context_namespace: String,
}

/// Builds a stable trial key from the experiment revision and the child label.
///
/// A validated declaration bounds its child labels by
/// [`MAX_CHILD_LABEL_BYTES`](super::label::MAX_CHILD_LABEL_BYTES), so a key derived here never
/// exceeds [`MAX_BRANCH_LABEL_BYTES`] and can always be recorded in a
/// [`BranchOutcome`](super::outcome::BranchOutcome).
#[must_use]
pub fn trial_key(revision: &str, child_label: &str) -> String {
    format!("{revision}{TRIAL_KEY_SEPARATOR}{child_label}")
}

/// Plans every logical trial, one per declared child, in declaration order.
///
/// # Errors
///
/// Returns the declaration rejection for an invalid or oversized declaration.
pub fn plan(
    manifest: &BranchExperimentManifest,
) -> Result<Vec<PlannedBranchTrial>, BranchExperimentError> {
    let revision = manifest.digest()?;
    Ok(manifest
        .children
        .iter()
        .map(|child| {
            let key = trial_key(&revision, &child.child_label);
            let context_namespace = format!("{CONTEXT_NAMESPACE_PREFIX}{key}");
            debug_assert!(
                key.len() <= MAX_BRANCH_LABEL_BYTES
                    && context_namespace.len() <= MAX_BRANCH_LABEL_BYTES,
                "a validated declaration must derive bounded trial labels"
            );
            PlannedBranchTrial {
                context_namespace,
                trial_key: key,
                child_label: child.child_label.clone(),
                first_action: child.first_action.clone(),
            }
        })
        .collect())
}
