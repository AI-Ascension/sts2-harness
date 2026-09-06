// SPDX-License-Identifier: MIT

mod schema;
mod store_checkpoint;
mod store_completion;
mod store_core;
mod store_core_helpers;
mod store_jobs;
mod store_ops;
mod store_provider;
mod store_provider_queries;
mod store_recovery;
mod store_recovery_attempt;
mod types;

pub use store_core::ExecutionStore;
pub use types::{
    AttemptKind, AttemptState, Checkpoint, CompletionRecord, CompletionStatus, DecisionReference,
    ExecutionFingerprint, ExecutionLineage, ExecutionStoreConfig, ExecutionStoreError, JobClaim,
    JobClaimOutcome, JobState, OperationIntent, OperationState, ProviderFailureClass,
    ProviderReservation, ProviderReservationState, RECOVERY_CONTRACT_VERSION,
    RECOVERY_SCHEMA_DIGEST, RecoveryDisposition, ResumeState, StorePragmas, StoredAttempt,
    StoredDecision, StoredEpisode, StoredJob, StoredOperation,
};
