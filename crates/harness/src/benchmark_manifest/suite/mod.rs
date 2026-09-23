// SPDX-License-Identifier: MIT

//! Source-only planning, scheduling and reporting for reproducible benchmark suites.
//!
//! A suite freezes one ordered seed corpus, one policy axis, a repetition count, an evaluator
//! revision, declared budgets and predeclared metrics under a versioned manifest. The owner API
//! covers every path an operator needs without launching a game or spending provider credit:
//!
//! - validate: [`SuiteManifest::validate`] and [`SuiteManifest::digest`];
//! - run: [`plan`] plus [`SuiteScheduler::start`]/[`SuiteScheduler::settle`];
//! - status: [`SuiteScheduler::pending`], [`SuiteScheduler::phase_of`] and
//!   [`SuiteScheduler::attempts`];
//! - resume: [`SuiteScheduler::resume`] replays recorded outcomes over the frozen plan;
//! - export: [`aggregate`] plus [`SuiteReport::to_json_pretty`].
//!
//! A different corpus, policy set, evaluator revision or budget set produces a different suite
//! revision, so prior results stay attached to the revision that produced them. Native exact-start
//! certification remains a separate gate: an unverified trial is excluded from exact-start groups
//! rather than blended into a certified statistic.

mod comparison;
mod error;
mod index;
mod manifest;
mod plan;
mod report;
mod results;
mod scheduler;
mod witness;

pub use comparison::{PairedComparison, compare_paired_policies};
pub use error::{ReportError, ScheduleError};
pub use manifest::{
    MAX_SUITE_CASES, MAX_SUITE_CONCURRENCY, MAX_SUITE_LABEL_BYTES, MAX_SUITE_MANIFEST_BYTES,
    MAX_SUITE_METRICS, MAX_SUITE_POLICIES, MAX_SUITE_REPETITIONS, PolicyConfig, SUITE_VERSION,
    SeedCase, SeedCorpus, SuiteBudgets, SuiteManifest, SuiteManifestError,
};
pub use plan::{CONTEXT_NAMESPACE_PREFIX, PlannedSuiteTrial, TRIAL_KEY_SEPARATOR, plan, trial_key};
pub use report::{
    CellReport, MetricAvailability, MetricCoverage, SuiteReport, aggregate, ensure_metric_coverage,
};
pub use results::{
    MAX_TRIAL_KEY_BYTES, Measurement, Metric, NativeWitness, TrialOutcome, TrialOutcomeError,
    TrialResult, TrialStatus,
};
pub use scheduler::{Settlement, SuiteScheduler, TrialPhase};
pub use witness::{StartGroup, StartMember, WitnessAudit, audit_initial_witness};
