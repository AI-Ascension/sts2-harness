// SPDX-License-Identifier: MIT

use super::Evaluator;

impl Evaluator {
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

fn rate(numerator: usize, denominator: usize) -> u64 {
    if denominator == 0 {
        0
    } else {
        numerator as u64 * 1000 / denominator as u64
    }
}
