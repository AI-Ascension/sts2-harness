// SPDX-License-Identifier: MIT

#[path = "types_core.rs"]
mod core;
#[path = "types_enums.rs"]
mod enums;
#[path = "types_error.rs"]
mod error;
#[path = "types_records.rs"]
mod records;

pub use core::{
    Checkpoint, ExecutionFingerprint, ExecutionLineage, ExecutionStoreConfig,
    RECOVERY_CONTRACT_VERSION, RECOVERY_SCHEMA_DIGEST, StorePragmas,
};
pub use enums::{
    AttemptKind, AttemptState, CompletionStatus, JobState, OperationState, ProviderFailureClass,
    ProviderReservationState, RecoveryDisposition,
};
pub use error::ExecutionStoreError;
pub use records::{
    CompletionRecord, DecisionReference, JobClaim, JobClaimOutcome, OperationIntent,
    ProviderReservation, ResumeState, StoredAttempt, StoredDecision, StoredEpisode, StoredJob,
    StoredOperation,
};

pub(crate) use core::{valid_digest, valid_id, valid_reference};
