// SPDX-License-Identifier: MIT

mod action_envelope;
mod schema;
mod schema_workflow;
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
mod store_workflow;
mod types;
mod workflow_types;

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
pub use workflow_types::{
    GameOperationId, InvocationOutcome, InvocationState, MAX_WORKFLOW_BYTES,
    MAX_WORKFLOW_COUNTER_NAME_BYTES, MAX_WORKFLOW_COUNTERS, MAX_WORKFLOW_CURSOR,
    MAX_WORKFLOW_STACK_DEPTH, RunProjection, RunStatus, StoredWorkflowInvocation,
    WORKFLOW_CONTRACT_VERSION, WorkflowCommandId, WorkflowDefinition, WorkflowDefinitionId,
    WorkflowEpisodeId, WorkflowEvent, WorkflowEventId, WorkflowEventPayload, WorkflowInvocation,
    WorkflowInvocationId, WorkflowPlan, WorkflowPlanId, WorkflowRunId, WorkflowRunSnapshot,
    WorkflowRunStart,
};
