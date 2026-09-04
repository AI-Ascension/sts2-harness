// SPDX-License-Identifier: MIT

use super::observation::EpisodeObservation;
use super::transition::{DispatchStatus, TransitionReceipt};

/// Independently verified effect facts required before the episode advances.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedTransition {
    before_generation: u64,
    after_generation: u64,
    state_id: String,
    effect_kind: String,
}

impl VerifiedTransition {
    #[must_use]
    pub const fn before_generation(&self) -> u64 {
        self.before_generation
    }

    #[must_use]
    pub const fn after_generation(&self) -> u64 {
        self.after_generation
    }

    #[must_use]
    pub fn state_id(&self) -> &str {
        &self.state_id
    }

    #[must_use]
    pub fn effect_kind(&self) -> &str {
        &self.effect_kind
    }
}

/// Verifies a settlement receipt without inferring a game effect from acknowledgement alone.
pub fn verify_settlement(
    before: &EpisodeObservation,
    receipt: &TransitionReceipt,
) -> Result<VerifiedTransition, PostconditionError> {
    match receipt.status() {
        DispatchStatus::Accepted => Err(PostconditionError::AdmissionOnly),
        DispatchStatus::Unknown => Err(PostconditionError::RecoveryRequired),
        DispatchStatus::Rejected | DispatchStatus::Cancelled => {
            Err(PostconditionError::ActionRejected)
        }
        DispatchStatus::Settled => {
            let after = receipt
                .after()
                .ok_or(PostconditionError::MissingObservation)?;
            let effect_kind = receipt
                .effect_kind()
                .ok_or(PostconditionError::MissingWitness)?;
            if after.generation() <= before.generation() {
                return Err(PostconditionError::StaleObservation);
            }
            Ok(VerifiedTransition {
                before_generation: before.generation(),
                after_generation: after.generation(),
                state_id: after.state_id().to_owned(),
                effect_kind: effect_kind.to_owned(),
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PostconditionError {
    AdmissionOnly,
    RecoveryRequired,
    ActionRejected,
    MissingObservation,
    MissingWitness,
    StaleObservation,
}

impl std::fmt::Display for PostconditionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::AdmissionOnly => "action was accepted but not settled",
            Self::RecoveryRequired => "action result is unknown and requires recovery",
            Self::ActionRejected => "action was rejected or cancelled",
            Self::MissingObservation => "settlement lacks a fresh observation",
            Self::MissingWitness => "settlement lacks an effect witness",
            Self::StaleObservation => "settlement lacks a fresh generation",
        })
    }
}

impl std::error::Error for PostconditionError {}
