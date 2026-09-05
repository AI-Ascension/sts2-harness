// SPDX-License-Identifier: MIT

#[path = "evaluation_report.rs"]
mod report;
pub use report::EvaluationReport;

const MAX_SAMPLES: usize = 65_536;

/// Terminal result used by the evaluator; victory and defeat are never combined.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalOutcome {
    Victory,
    Defeat,
}

/// One bounded evaluation observation. All values are supplied by a caller with evidence lineage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvaluationSample {
    pub legal: bool,
    pub stale: bool,
    pub verified: bool,
    pub recovery_attempted: bool,
    pub recovery_succeeded: bool,
    pub regret_millis: Option<u64>,
    pub confidence_percent: Option<u8>,
    pub outcome_success: Option<bool>,
    pub request_bytes: u32,
    pub response_bytes: u32,
    pub latency_millis: u32,
    pub progressed: bool,
    pub terminal: Option<TerminalOutcome>,
}

impl Default for EvaluationSample {
    fn default() -> Self {
        Self {
            legal: true,
            stale: false,
            verified: false,
            recovery_attempted: false,
            recovery_succeeded: false,
            regret_millis: None,
            confidence_percent: None,
            outcome_success: None,
            request_bytes: 0,
            response_bytes: 0,
            latency_millis: 0,
            progressed: false,
            terminal: None,
        }
    }
}

/// Incremental, overflow-checked evaluator for one bounded episode or cohort.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Evaluator {
    capacity: usize,
    samples: usize,
    legal: usize,
    illegal: usize,
    stale: usize,
    verified: usize,
    unverified: usize,
    recovery_attempts: usize,
    recovery_successes: usize,
    regret_samples: usize,
    regret_sum_millis: u64,
    calibration_samples: usize,
    calibration_error_sum_percent: u64,
    resource_calls: usize,
    request_bytes: u64,
    response_bytes: u64,
    latency_millis: u64,
    progression_steps: usize,
    victories: usize,
    defeats: usize,
}

impl Evaluator {
    pub fn new(capacity: usize) -> Result<Self, EvaluationError> {
        if capacity == 0 || capacity > MAX_SAMPLES {
            return Err(EvaluationError::InvalidCapacity);
        }
        Ok(Self {
            capacity,
            samples: 0,
            legal: 0,
            illegal: 0,
            stale: 0,
            verified: 0,
            unverified: 0,
            recovery_attempts: 0,
            recovery_successes: 0,
            regret_samples: 0,
            regret_sum_millis: 0,
            calibration_samples: 0,
            calibration_error_sum_percent: 0,
            resource_calls: 0,
            request_bytes: 0,
            response_bytes: 0,
            latency_millis: 0,
            progression_steps: 0,
            victories: 0,
            defeats: 0,
        })
    }

    pub fn observe(&mut self, sample: EvaluationSample) -> Result<(), EvaluationError> {
        let mut candidate = self.clone();
        candidate.observe_checked(sample)?;
        *self = candidate;
        Ok(())
    }

    fn observe_checked(&mut self, sample: EvaluationSample) -> Result<(), EvaluationError> {
        if self.samples >= self.capacity {
            return Err(EvaluationError::Full);
        }
        if let Some(confidence) = sample.confidence_percent
            && confidence > 100
        {
            return Err(EvaluationError::InvalidConfidence);
        }
        self.samples = add(self.samples, 1)?;
        if sample.legal {
            self.legal = add(self.legal, 1)?;
        } else {
            self.illegal = add(self.illegal, 1)?;
        }
        if sample.stale {
            self.stale = add(self.stale, 1)?;
        }
        if sample.verified {
            self.verified = add(self.verified, 1)?;
        } else {
            self.unverified = add(self.unverified, 1)?;
        }
        if sample.recovery_attempted {
            self.recovery_attempts = add(self.recovery_attempts, 1)?;
            if sample.recovery_succeeded {
                self.recovery_successes = add(self.recovery_successes, 1)?;
            }
        }
        if let Some(regret) = sample.regret_millis {
            self.regret_samples = add(self.regret_samples, 1)?;
            self.regret_sum_millis = self
                .regret_sum_millis
                .checked_add(regret)
                .ok_or(EvaluationError::Overflow)?;
        }
        if let (Some(confidence), Some(success)) =
            (sample.confidence_percent, sample.outcome_success)
        {
            let expected = if success { 100_u8 } else { 0_u8 };
            self.calibration_samples = add(self.calibration_samples, 1)?;
            self.calibration_error_sum_percent = self
                .calibration_error_sum_percent
                .checked_add(u64::from(confidence.abs_diff(expected)))
                .ok_or(EvaluationError::Overflow)?;
        }
        if sample.request_bytes > 0 || sample.response_bytes > 0 || sample.latency_millis > 0 {
            self.resource_calls = add(self.resource_calls, 1)?;
        }
        self.request_bytes = self
            .request_bytes
            .checked_add(u64::from(sample.request_bytes))
            .ok_or(EvaluationError::Overflow)?;
        self.response_bytes = self
            .response_bytes
            .checked_add(u64::from(sample.response_bytes))
            .ok_or(EvaluationError::Overflow)?;
        self.latency_millis = self
            .latency_millis
            .checked_add(u64::from(sample.latency_millis))
            .ok_or(EvaluationError::Overflow)?;
        if sample.progressed {
            self.progression_steps = add(self.progression_steps, 1)?;
        }
        match sample.terminal {
            Some(TerminalOutcome::Victory) => self.victories = add(self.victories, 1)?,
            Some(TerminalOutcome::Defeat) => self.defeats = add(self.defeats, 1)?,
            None => {}
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvaluationError {
    InvalidCapacity,
    Full,
    InvalidConfidence,
    Overflow,
}

impl std::fmt::Display for EvaluationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidCapacity => "evaluation capacity is invalid",
            Self::Full => "evaluation sample capacity is exhausted",
            Self::InvalidConfidence => "confidence must be between 0 and 100 percent",
            Self::Overflow => "evaluation counter overflowed",
        })
    }
}

impl std::error::Error for EvaluationError {}

fn add(value: usize, increment: usize) -> Result<usize, EvaluationError> {
    value
        .checked_add(increment)
        .ok_or(EvaluationError::Overflow)
}
