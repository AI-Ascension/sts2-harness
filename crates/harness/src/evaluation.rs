// SPDX-License-Identifier: MIT

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

    #[must_use]
    pub fn report(&self) -> EvaluationReport {
        EvaluationReport {
            samples: self.samples,
            legal: self.legal,
            illegal: self.illegal,
            stale: self.stale,
            verified: self.verified,
            unverified: self.unverified,
            recovery_attempts: self.recovery_attempts,
            recovery_successes: self.recovery_successes,
            regret_samples: self.regret_samples,
            regret_sum_millis: self.regret_sum_millis,
            calibration_samples: self.calibration_samples,
            calibration_error_sum_percent: self.calibration_error_sum_percent,
            resource_calls: self.resource_calls,
            request_bytes: self.request_bytes,
            response_bytes: self.response_bytes,
            latency_millis: self.latency_millis,
            progression_steps: self.progression_steps,
            victories: self.victories,
            defeats: self.defeats,
        }
    }
}

/// Snapshot of episode-quality metrics, using integer rates to keep replay deterministic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvaluationReport {
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

impl EvaluationReport {
    #[must_use]
    pub const fn samples(self) -> usize {
        self.samples
    }
    #[must_use]
    pub const fn legal(self) -> usize {
        self.legal
    }
    #[must_use]
    pub const fn illegal(self) -> usize {
        self.illegal
    }
    #[must_use]
    pub const fn stale(self) -> usize {
        self.stale
    }
    #[must_use]
    pub const fn verified(self) -> usize {
        self.verified
    }
    #[must_use]
    pub const fn unverified(self) -> usize {
        self.unverified
    }
    #[must_use]
    pub const fn recovery_attempts(self) -> usize {
        self.recovery_attempts
    }
    #[must_use]
    pub const fn recovery_successes(self) -> usize {
        self.recovery_successes
    }
    #[must_use]
    pub const fn regret_samples(self) -> usize {
        self.regret_samples
    }
    #[must_use]
    pub const fn regret_sum_millis(self) -> u64 {
        self.regret_sum_millis
    }
    #[must_use]
    pub const fn calibration_samples(self) -> usize {
        self.calibration_samples
    }
    #[must_use]
    pub const fn resource_calls(self) -> usize {
        self.resource_calls
    }
    #[must_use]
    pub const fn request_bytes(self) -> u64 {
        self.request_bytes
    }
    #[must_use]
    pub const fn response_bytes(self) -> u64 {
        self.response_bytes
    }
    #[must_use]
    pub const fn latency_millis(self) -> u64 {
        self.latency_millis
    }
    #[must_use]
    pub const fn progression_steps(self) -> usize {
        self.progression_steps
    }
    #[must_use]
    pub const fn victories(self) -> usize {
        self.victories
    }
    #[must_use]
    pub const fn defeats(self) -> usize {
        self.defeats
    }

    #[must_use]
    pub fn legality_rate_millis(self) -> u64 {
        rate(self.legal, self.samples)
    }

    #[must_use]
    pub fn verification_rate_millis(self) -> u64 {
        rate(self.verified, self.samples)
    }

    #[must_use]
    pub fn recovery_success_rate_millis(self) -> u64 {
        rate(self.recovery_successes, self.recovery_attempts)
    }

    #[must_use]
    pub fn mean_regret_millis(self) -> Option<u64> {
        (self.regret_samples > 0).then(|| self.regret_sum_millis / self.regret_samples as u64)
    }

    #[must_use]
    pub fn calibration_error_percent(self) -> Option<u64> {
        (self.calibration_samples > 0)
            .then(|| self.calibration_error_sum_percent / self.calibration_samples as u64)
    }

    #[must_use]
    pub const fn completed(self) -> bool {
        self.victories + self.defeats == 1
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

fn rate(numerator: usize, denominator: usize) -> u64 {
    if denominator == 0 {
        0
    } else {
        numerator as u64 * 1000 / denominator as u64
    }
}
