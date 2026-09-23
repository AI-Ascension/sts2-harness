// SPDX-License-Identifier: MIT

//! Source-only cold-launch trial isolation over a pristine profile baseline.
//!
//! A trial is admitted against an immutable baseline, given a fresh exclusively leased
//! destination, provisioned from that baseline, launched as a newly attested native process,
//! driven through setup, run, stop and cleanup with every stage recorded before the next, and
//! reconciled on a lost reply. No game is launched here: the port that performs effects belongs
//! to the gateway, and this module fixes the contract and the effect-free failure fixtures.

mod baseline;
mod error;
mod evidence;
mod lease;
mod lifecycle;
mod orchestrator;
mod process;
mod stage;

pub use baseline::{
    BaselineError, BaselineMismatch, LaunchProfile, MAX_BASELINE_EXCLUSIONS,
    MAX_BASELINE_LABEL_BYTES, PristineBaseline, TelemetryExclusions,
};
pub use error::ColdLaunchError;
pub use evidence::{ColdStartEvidence, evidence_of};
pub use lease::{Destination, LeaseAllocator, MAX_CONCURRENT_ALLOCATIONS};
pub use lifecycle::TrialLifecycle;
pub use orchestrator::ColdLaunchOrchestrator;
pub use process::{MAX_PROCESS_TOKEN_BYTES, ProcessBirth, ProcessError, ReadinessProof};
pub use stage::{ColdLaunchStage, MAX_COLD_TRIAL_KEY_BYTES};
