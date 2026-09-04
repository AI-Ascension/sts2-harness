// SPDX-License-Identifier: MIT

use super::observation::EpisodeObservation;

/// Result returned by a gateway/MCP wait operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitOutcome {
    Successor,
    SameStateMutation,
    Timeout,
    RecoveryRequired,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WaitSample {
    outcome: WaitOutcome,
    observation: Option<EpisodeObservation>,
    effect_kind: Option<String>,
}

impl WaitSample {
    #[must_use]
    pub const fn new(outcome: WaitOutcome, observation: Option<EpisodeObservation>) -> Self {
        Self {
            outcome,
            observation,
            effect_kind: None,
        }
    }

    #[must_use]
    pub fn with_effect_kind(mut self, effect_kind: impl Into<String>) -> Self {
        self.effect_kind = Some(effect_kind.into());
        self
    }

    #[must_use]
    pub const fn outcome(&self) -> WaitOutcome {
        self.outcome
    }

    #[must_use]
    pub fn observation(&self) -> Option<&EpisodeObservation> {
        self.observation.as_ref()
    }

    #[must_use]
    pub fn effect_kind(&self) -> Option<&str> {
        self.effect_kind.as_deref()
    }
}

/// Port used by the barrier; implementations own transport and timing.
pub trait BarrierPort {
    fn wait_for_transition(
        &mut self,
        operation_id: &str,
        wait_for_millis: u32,
    ) -> Result<WaitSample, BarrierError>;
}

/// Semantic barrier using bounded polling, never a global sleep.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StabilityBarrier {
    max_polls: u8,
    wait_for_millis: u32,
}

impl StabilityBarrier {
    pub fn new(max_polls: u8, wait_for_millis: u32) -> Result<Self, BarrierError> {
        if max_polls == 0 || wait_for_millis == 0 || wait_for_millis > 120_000 {
            return Err(BarrierError::InvalidConfiguration);
        }
        Ok(Self {
            max_polls,
            wait_for_millis,
        })
    }

    pub fn await_transition<P: BarrierPort>(
        &self,
        port: &mut P,
        operation_id: &str,
        before: &EpisodeObservation,
    ) -> Result<EpisodeObservation, BarrierError> {
        Ok(self
            .await_transition_sample(port, operation_id, before)?
            .observation
            .ok_or(BarrierError::MissingObservation)?)
    }

    pub fn await_transition_sample<P: BarrierPort>(
        &self,
        port: &mut P,
        operation_id: &str,
        before: &EpisodeObservation,
    ) -> Result<WaitSample, BarrierError> {
        if operation_id.is_empty() {
            return Err(BarrierError::InvalidOperation);
        }
        for _ in 0..self.max_polls {
            let sample = port.wait_for_transition(operation_id, self.wait_for_millis)?;
            match sample.outcome() {
                WaitOutcome::Successor | WaitOutcome::SameStateMutation => {
                    let observation = sample
                        .observation()
                        .ok_or(BarrierError::MissingObservation)?;
                    if observation.generation() > before.generation() {
                        return Ok(sample);
                    }
                    return Err(BarrierError::StaleObservation);
                }
                WaitOutcome::RecoveryRequired => return Err(BarrierError::RecoveryRequired),
                WaitOutcome::Timeout => {}
            }
        }
        Err(BarrierError::Timeout)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BarrierError {
    InvalidConfiguration,
    InvalidOperation,
    MissingObservation,
    StaleObservation,
    RecoveryRequired,
    Timeout,
    PortFailure,
}

impl std::fmt::Display for BarrierError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidConfiguration => "stability barrier configuration is invalid",
            Self::InvalidOperation => "stability barrier operation identity is invalid",
            Self::MissingObservation => "transition wait lacked an observation",
            Self::StaleObservation => "transition wait returned a stale observation",
            Self::RecoveryRequired => "transition wait requires recovery",
            Self::Timeout => "transition wait timed out",
            Self::PortFailure => "transition wait port failed",
        })
    }
}

impl std::error::Error for BarrierError {}
