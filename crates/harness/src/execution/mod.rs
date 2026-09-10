// SPDX-License-Identifier: MIT

mod action_envelope;
mod schema;
mod store_checkpoint;
mod store_completion;
mod store_core;
mod store_core_helpers;
mod store_jobs;
mod store_operation_queries;
mod store_ops;
mod store_provider;
mod store_provider_queries;
mod store_provider_results;
mod store_recovery;
mod store_recovery_attempt;
mod store_recovery_disposition;
mod types;

pub use store_core::ExecutionStore;
pub use types::{
    AttemptKind, AttemptState, CatalogEvidence, Checkpoint, CompletionRecord, CompletionStatus,
    DecisionReference, ExecutionFingerprint, ExecutionLineage, ExecutionStoreConfig,
    ExecutionStoreError, JobClaim, JobClaimOutcome, JobState, MAX_CATALOG_BYTES,
    MAX_OPERATION_ACTION_BYTES, OperationIntent, OperationState, ProviderFailureClass,
    ProviderReservation, ProviderReservationState, RECOVERY_CONTRACT_VERSION,
    RECOVERY_SCHEMA_DIGEST, RecoveryDisposition, ResumeState, StorePragmas, StoredAttempt,
    StoredDecision, StoredEpisode, StoredJob, StoredOperation,
};
