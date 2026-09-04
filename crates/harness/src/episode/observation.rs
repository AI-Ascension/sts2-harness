// SPDX-License-Identifier: MIT

use crate::exo::SanitizedObservation;
use serde_json::Value;

const MAX_STATE_ID_BYTES: usize = 512;

/// Player-visible stage used by the episode state machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EpisodeStage {
    Setup,
    Map,
    Combat,
    Reward,
    Shop,
    Event,
    Rest,
    Selection,
    Victory,
    Defeat,
    Recovery,
    Unknown,
}

impl EpisodeStage {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Victory | Self::Defeat)
    }

    #[must_use]
    pub const fn is_actionable(self) -> bool {
        !matches!(self, Self::Victory | Self::Defeat | Self::Recovery | Self::Unknown)
    }
}

/// Owned observation plus explicit actionability facts from the MCP/game-mod boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpisodeObservation {
    state_id: String,
    generation: u64,
    stage: EpisodeStage,
    actionable: bool,
    modal_blocking: bool,
    input_enabled: bool,
    fair_play: SanitizedObservation,
}

impl EpisodeObservation {
    /// Validates the ordinary projection before it enters policy code.
    pub fn new(
        state_id: impl Into<String>,
        generation: u64,
        stage: EpisodeStage,
        actionable: bool,
        modal_blocking: bool,
        input_enabled: bool,
        fair_play: Value,
    ) -> Result<Self, ObservationError> {
        let state_id = state_id.into();
        if state_id.is_empty()
            || state_id.len() > MAX_STATE_ID_BYTES
            || !state_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
        {
            return Err(ObservationError::InvalidIdentity);
        }
        if generation > 9_007_199_254_740_991 {
            return Err(ObservationError::InvalidGeneration);
        }
        if stage == EpisodeStage::Unknown || stage == EpisodeStage::Recovery {
            if actionable {
                return Err(ObservationError::UnknownState);
            }
        }
        let fair_play = SanitizedObservation::new(fair_play)
            .map_err(|_| ObservationError::PrivilegedProjection)?;
        if fair_play.state_id() != Some(state_id.as_str())
            || fair_play.generation() != Some(generation)
        {
            return Err(ObservationError::ProjectionMismatch);
        }
        Ok(Self {
            state_id,
            generation,
            stage,
            actionable,
            modal_blocking,
            input_enabled,
            fair_play,
        })
    }

    #[must_use]
    pub fn state_id(&self) -> &str {
        &self.state_id
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn stage(&self) -> EpisodeStage {
        self.stage
    }

    #[must_use]
    pub const fn actionable(&self) -> bool {
        self.actionable && self.stage.is_actionable()
    }

    #[must_use]
    pub const fn modal_blocking(&self) -> bool {
        self.modal_blocking
    }

    #[must_use]
    pub const fn input_enabled(&self) -> bool {
        self.input_enabled
    }

    #[must_use]
    pub fn fair_play(&self) -> &SanitizedObservation {
        &self.fair_play
    }

    /// Returns whether policy may dispatch against this snapshot.
    pub fn assert_actionable(&self) -> Result<(), EpisodeError> {
        if self.stage == EpisodeStage::Unknown {
            return Err(EpisodeError::UnknownState);
        }
        if self.modal_blocking || !self.input_enabled || !self.actionable() {
            return Err(EpisodeError::InputBlocked);
        }
        Ok(())
    }
}

/// Observation construction failures are intentionally non-retryable until a fresh projection exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationError {
    InvalidIdentity,
    InvalidGeneration,
    UnknownState,
    PrivilegedProjection,
    ProjectionMismatch,
}

impl std::fmt::Display for ObservationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentity => "observation state identity is invalid",
            Self::InvalidGeneration => "observation generation is out of bounds",
            Self::UnknownState => "unknown/recovery observation cannot be actionable",
            Self::PrivilegedProjection => "observation failed the fair-play firewall",
            Self::ProjectionMismatch => "observation projection identity does not match its envelope",
        })
    }
}

impl std::error::Error for ObservationError {}
