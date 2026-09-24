// SPDX-License-Identifier: MIT

//! Source-only declaration, scheduling and comparison of bounded same-start branch experiments.
//!
//! An experiment forks one verified exact checkpoint into a bounded set of child policies and
//! compares their recorded transitions honestly. It launches no game, restores nothing and invokes
//! no provider: the restore, the child processes and the provider calls belong to the gateway and
//! game-mod, and this module fixes the effect-free contract those live paths must satisfy.
//!
//! The owner API is deliberately small:
//!
//! - declare: [`BranchExperimentManifest`] freezes the fork point, child policies, strategy, stop
//!   conditions and per-child/total budgets, and [`plan`] derives one stable trial per child;
//! - admit: [`admit_start`] re-checks same-start admission for each trial;
//! - run: [`BranchExperimentScheduler::start`]/[`BranchExperimentScheduler::settle`]/
//!   [`BranchExperimentScheduler::cancel`]/[`BranchExperimentScheduler::resume`];
//! - compare: [`compare_branches`] aligns by logical action and separates policy divergence from
//!   restore failure;
//! - export: [`aggregate`] and [`BranchExperimentManifest::public_projection`], which carry a
//!   keyed handle and no exact digest.

mod admission;
mod comparison;
mod declaration;
mod error;
mod outcome;
mod plan;
mod report;
mod scheduler;

pub use admission::{StartAdmission, admit_start, is_verified_start};
pub use comparison::{aggregate, compare_branches};
pub use declaration::{
    BRANCH_EXPERIMENT_VERSION, BranchBudgets, BranchExperimentManifest, BranchExperimentPublic,
    CONTEXT_NAMESPACE_PREFIX, ChildPolicy, ForkStrategy, MAX_BRANCH_CHILDREN,
    MAX_BRANCH_CONCURRENCY, MAX_BRANCH_LABEL_BYTES, MAX_MANIFEST_BYTES, StopCondition,
    TRIAL_KEY_SEPARATOR,
};
pub use error::{AdmissionError, BranchExperimentError, ComparisonError};
pub use outcome::{BranchOutcome, BranchStatus};
pub use plan::{PlannedBranchTrial, plan, trial_key};
pub use report::{BranchComparison, BranchDivergence, BranchExperimentReport};
pub use scheduler::{BranchExperimentScheduler, ScheduleError, Settlement, TrialPhase};
