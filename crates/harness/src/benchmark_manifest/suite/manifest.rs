// SPDX-License-Identifier: MIT

//! Versioned, immutable benchmark-suite declarations over a frozen seed corpus.
//!
//! A suite manifest names the benchmark it evaluates, the ordered seed corpus, the model/policy
//! configurations, the repetition count, the evaluator revision, the declared budgets and the
//! predeclared metrics. It is a declaration only: it neither launches a game nor spends provider
//! credit. Corpus/order randomization, the native game seed and the optional provider sampling seed
//! are separate typed fields and are never substituted for one another.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Only supported suite-manifest version; unknown semantics require a new version.
pub const SUITE_VERSION: &str = "ascension.benchmark-suite.v1";
/// Whole manifest byte bound, checked before the digest is computed.
pub const MAX_SUITE_MANIFEST_BYTES: usize = 64 * 1024;
/// Maximum distinct seed cases in one corpus.
pub const MAX_SUITE_CASES: usize = 256;
/// Maximum policy configurations in one suite.
pub const MAX_SUITE_POLICIES: usize = 32;
/// Maximum repetitions of one (case, policy) pair.
pub const MAX_SUITE_REPETITIONS: u32 = 64;
/// Maximum concurrent trials declared for one suite.
pub const MAX_SUITE_CONCURRENCY: usize = 32;
/// Maximum bytes in a case, policy, benchmark or evaluator label.
pub const MAX_SUITE_LABEL_BYTES: usize = 128;
/// Maximum predeclared metrics.
pub const MAX_SUITE_METRICS: usize = 32;

/// Rejection reasons for a suite manifest or the work it declares.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuiteManifestError {
    /// The declared version is not [`SUITE_VERSION`].
    UnsupportedVersion,
    /// The benchmark reference is empty, oversized or contains a NUL separator.
    InvalidBenchmarkRef,
    /// The evaluator revision is empty, oversized or contains a NUL separator.
    InvalidEvaluatorRevision,
    /// A case, policy, metric or settings label is empty, oversized or contains a NUL separator.
    InvalidLabel,
    /// The corpus declares no case.
    EmptyCorpus,
    /// The corpus exceeds [`MAX_SUITE_CASES`].
    TooManyCases,
    /// Two cases share a `case_id`.
    DuplicateCase,
    /// The suite declares no policy.
    EmptyPolicies,
    /// The suite exceeds [`MAX_SUITE_POLICIES`].
    TooManyPolicies,
    /// Two policies share a `policy_id`.
    DuplicatePolicy,
    /// A policy settings digest is empty, oversized or contains a NUL separator.
    InvalidSettingsDigest,
    /// The repetition count is zero or exceeds [`MAX_SUITE_REPETITIONS`].
    InvalidRepetitions,
    /// The suite declares no metric.
    EmptyMetrics,
    /// The suite exceeds [`MAX_SUITE_METRICS`].
    TooManyMetrics,
    /// Two metrics share a name.
    DuplicateMetric,
    /// A budget bound is zero, or the concurrency bound is outside `1..=MAX_SUITE_CONCURRENCY`.
    InvalidBudget,
    /// The declared axes would overflow the planned trial count.
    PlanOverflow,
    /// The canonical encoding exceeded [`MAX_SUITE_MANIFEST_BYTES`].
    TooLarge,
    /// The manifest could not be encoded canonically.
    NotEncodable,
}

impl fmt::Display for SuiteManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::UnsupportedVersion => "unsupported suite version",
            Self::InvalidBenchmarkRef => "invalid benchmark reference",
            Self::InvalidEvaluatorRevision => "invalid evaluator revision",
            Self::InvalidLabel => "invalid label",
            Self::EmptyCorpus => "empty seed corpus",
            Self::TooManyCases => "too many seed cases",
            Self::DuplicateCase => "duplicate seed case",
            Self::EmptyPolicies => "empty policy axis",
            Self::TooManyPolicies => "too many policies",
            Self::DuplicatePolicy => "duplicate policy",
            Self::InvalidSettingsDigest => "invalid settings digest",
            Self::InvalidRepetitions => "invalid repetition count",
            Self::EmptyMetrics => "empty metric set",
            Self::TooManyMetrics => "too many metrics",
            Self::DuplicateMetric => "duplicate metric",
            Self::InvalidBudget => "invalid budget",
            Self::PlanOverflow => "planned trial count overflow",
            Self::TooLarge => "suite manifest exceeds the byte bound",
            Self::NotEncodable => "suite manifest is not encodable",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for SuiteManifestError {}

/// One immutable seed case: a stable identifier bound to the native game seed it starts.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SeedCase {
    /// Stable case identifier, unique within the corpus.
    pub case_id: String,
    /// Native game seed for this case; never substituted with a randomization seed.
    pub native_game_seed: u64,
}

/// Ordered seed corpus with distinct randomization seeds.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SeedCorpus {
    /// Ordered cases; the order is part of the frozen corpus.
    pub cases: Vec<SeedCase>,
    /// Seed for corpus/order randomization, separate from every native game seed.
    pub corpus_randomization_seed: u64,
    /// Optional provider sampling seed, separate from game and corpus seeds.
    pub provider_sampling_seed: u64,
}

/// One model/policy configuration axis value.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PolicyConfig {
    /// Stable policy identifier, unique within the suite.
    pub policy_id: String,
    /// Digest of the provider, prompt, workflow and context settings used by this policy.
    pub settings_digest: String,
}

/// Declared, non-negotiable budgets for one suite and each of its trials.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SuiteBudgets {
    /// Maximum settled actions admitted for one trial.
    pub max_steps_per_trial: u64,
    /// Maximum settled actions admitted across the whole suite.
    pub max_total_steps: u64,
    /// Maximum measured latency admitted across the whole suite.
    pub max_total_duration_millis: u64,
    /// Maximum concurrent trials; the pure scheduler is serial, the executor owns the width.
    pub max_concurrency: usize,
    /// Optional measured provider-spend cap in micros; `None` declares no spend bound.
    pub max_provider_spend_micros: Option<u64>,
}

/// A validated immutable suite declaration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SuiteManifest {
    /// Version tag; must equal [`SUITE_VERSION`].
    pub version: String,
    /// Reference to the benchmark declaration this suite evaluates.
    pub benchmark_ref: String,
    /// Frozen ordered seed corpus.
    pub corpus: SeedCorpus,
    /// Ordered policy axis.
    pub policies: Vec<PolicyConfig>,
    /// Repetitions of each (case, policy) pair.
    pub repetitions: u32,
    /// Evaluator revision the report belongs to.
    pub evaluator_revision: String,
    /// Declared budgets.
    pub budgets: SuiteBudgets,
    /// Predeclared metric names.
    pub metrics: Vec<String>,
}

impl SuiteManifest {
    /// Validates every declared axis, bound and label.
    ///
    /// # Errors
    ///
    /// Returns the specific [`SuiteManifestError`] for the first failing axis.
    pub fn validate(&self) -> Result<(), SuiteManifestError> {
        if self.version != SUITE_VERSION {
            return Err(SuiteManifestError::UnsupportedVersion);
        }
        if !label_ok(&self.benchmark_ref, 256) {
            return Err(SuiteManifestError::InvalidBenchmarkRef);
        }
        if !label_ok(&self.evaluator_revision, 256) {
            return Err(SuiteManifestError::InvalidEvaluatorRevision);
        }
        self.validate_corpus()?;
        self.validate_policies()?;
        if self.repetitions == 0 || self.repetitions > MAX_SUITE_REPETITIONS {
            return Err(SuiteManifestError::InvalidRepetitions);
        }
        self.validate_metrics()?;
        self.validate_budgets()?;
        self.planned_count().map(|_| ())
    }

    fn validate_corpus(&self) -> Result<(), SuiteManifestError> {
        if self.corpus.cases.is_empty() {
            return Err(SuiteManifestError::EmptyCorpus);
        }
        if self.corpus.cases.len() > MAX_SUITE_CASES {
            return Err(SuiteManifestError::TooManyCases);
        }
        let mut seen = std::collections::BTreeSet::new();
        for case in &self.corpus.cases {
            if !label_ok(&case.case_id, MAX_SUITE_LABEL_BYTES) {
                return Err(SuiteManifestError::InvalidLabel);
            }
            if !seen.insert(case.case_id.as_str()) {
                return Err(SuiteManifestError::DuplicateCase);
            }
        }
        Ok(())
    }

    fn validate_policies(&self) -> Result<(), SuiteManifestError> {
        if self.policies.is_empty() {
            return Err(SuiteManifestError::EmptyPolicies);
        }
        if self.policies.len() > MAX_SUITE_POLICIES {
            return Err(SuiteManifestError::TooManyPolicies);
        }
        let mut seen = std::collections::BTreeSet::new();
        for policy in &self.policies {
            if !label_ok(&policy.policy_id, MAX_SUITE_LABEL_BYTES) {
                return Err(SuiteManifestError::InvalidLabel);
            }
            if !label_ok(&policy.settings_digest, 256) {
                return Err(SuiteManifestError::InvalidSettingsDigest);
            }
            if !seen.insert(policy.policy_id.as_str()) {
                return Err(SuiteManifestError::DuplicatePolicy);
            }
        }
        Ok(())
    }

    fn validate_metrics(&self) -> Result<(), SuiteManifestError> {
        if self.metrics.is_empty() {
            return Err(SuiteManifestError::EmptyMetrics);
        }
        if self.metrics.len() > MAX_SUITE_METRICS {
            return Err(SuiteManifestError::TooManyMetrics);
        }
        let mut seen = std::collections::BTreeSet::new();
        for metric in &self.metrics {
            if !label_ok(metric, MAX_SUITE_LABEL_BYTES) {
                return Err(SuiteManifestError::InvalidLabel);
            }
            if !seen.insert(metric.as_str()) {
                return Err(SuiteManifestError::DuplicateMetric);
            }
        }
        Ok(())
    }

    fn validate_budgets(&self) -> Result<(), SuiteManifestError> {
        let budgets = self.budgets;
        if budgets.max_steps_per_trial == 0
            || budgets.max_total_steps == 0
            || budgets.max_total_duration_millis == 0
            || budgets.max_concurrency == 0
            || budgets.max_concurrency > MAX_SUITE_CONCURRENCY
            || budgets.max_provider_spend_micros == Some(0)
        {
            return Err(SuiteManifestError::InvalidBudget);
        }
        Ok(())
    }

    /// Counts the logical trials this suite plans: cases x policies x repetitions.
    ///
    /// # Errors
    ///
    /// Returns [`SuiteManifestError::PlanOverflow`] when the product overflows `u64`.
    pub fn planned_count(&self) -> Result<u64, SuiteManifestError> {
        let cases =
            u64::try_from(self.corpus.cases.len()).map_err(|_| SuiteManifestError::PlanOverflow)?;
        let policies =
            u64::try_from(self.policies.len()).map_err(|_| SuiteManifestError::PlanOverflow)?;
        let repetitions = u64::from(self.repetitions);
        cases
            .checked_mul(policies)
            .and_then(|value| value.checked_mul(repetitions))
            .ok_or(SuiteManifestError::PlanOverflow)
    }

    /// Returns the canonical SHA-256 digest used as the suite revision identity.
    ///
    /// # Errors
    ///
    /// Returns [`SuiteManifestError::NotEncodable`] or [`SuiteManifestError::TooLarge`].
    pub fn digest(&self) -> Result<String, SuiteManifestError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| SuiteManifestError::NotEncodable)?;
        if bytes.len() > MAX_SUITE_MANIFEST_BYTES {
            return Err(SuiteManifestError::TooLarge);
        }
        Ok(crate::sha256_hex(&bytes))
    }
}

fn label_ok(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.contains('\0')
}
