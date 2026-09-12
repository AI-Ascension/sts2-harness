// SPDX-License-Identifier: MIT

use sha2::{Digest as _, Sha256};

/// Encode bytes as lowercase hexadecimal without relying on `LowerHex` support
/// from the digest type.
#[must_use]
pub fn hex_bytes(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

/// Return the lowercase hexadecimal SHA-256 digest of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: impl AsRef<[u8]>) -> String {
    hex_bytes(Sha256::digest(bytes))
}

mod artifact;
mod checkpoint_capability;
mod checkpoint_projection;
mod checkpoint_session;
mod checkpoint_verify;
mod context_capture;
pub use checkpoint_capability::{
    CapabilityError, CapabilityReport, CaptureFailure, CheckpointEvidence, CheckpointMode,
    PublicEvidenceSummary, RestoreFailure, UnsupportedPhase,
};
pub use checkpoint_projection::{
    HANDLE_DOMAIN, HANDLE_PREFIX, MIN_HANDLE_KEY_BYTES, ProjectionError, ProjectionKey,
    PublicCheckpointSummary,
};
pub use checkpoint_session::{SessionAdmission, SessionError, admit_session};
pub use checkpoint_verify::{VerificationFailure, VerificationOutcome, verify_checkpoint};
pub mod context_control;
pub mod context_memory;
mod coop_native;
mod coordinator;
mod decision_records;
mod episode;
mod error;
mod evaluation;
mod exact_transition;
mod execution;
mod execution_cancellation;
mod exo;
mod exo_process;
mod identity;
pub mod management;
mod map;
mod memory;
mod operation_journal;
pub mod phase3_adapter_demo;
mod poc;
mod protocol_artifact;
mod protocol_artifact_coop_receipt_query;
mod provider;
pub mod provider_session;
#[cfg(unix)]
pub use operation_journal::{
    JournalDecision, JournalEntry, JournalError, JournalKey, JournalOutcome, MAX_JOURNAL_ENTRIES,
    MAX_JOURNAL_FIELD_BYTES, OperationJournal,
};
pub mod recorded_run;
mod records;
mod replay;
mod restore_gate;
mod routing;
mod runtime_v2;
mod runtime_v2_artifact;
mod runtime_v4_expert;
mod runtime_v4_expert_action;
mod runtime_v4_expert_action_artifact;
mod runtime_v4_expert_artifact;
mod runtime_v4_expert_rest_action;
mod runtime_v4_expert_rest_action_artifact;

pub mod worker_endpoint;
pub mod worker_handoff;
pub mod worker_runtime;
pub mod worker_runtime_store;

pub mod workflow;

pub use artifact::{
    ArtifactDraft, ArtifactKind, ArtifactLineage, ArtifactMetadata, ArtifactMetadataInput,
    ArtifactPort, ArtifactPublicationRequest, ArtifactReceipt,
};
pub use context_capture::{
    CaptureBoundary, CaptureComponent, CaptureComponentKind, CaptureError, CaptureInput,
    CaptureMode, CapturePort, CaptureRecord, MAX_CAPTURE_BYTES, MAX_CAPTURE_RECORDS, MemoryCapture,
    NoopCapture, PreparedAstraInput, PreparedInput, PreparedOllamaInput, TransportState,
    generated_capture_attempt_id,
};
pub use context_control::*;
pub use coop_native::*;
pub use coordinator::{EpisodeHandle, Harness, HarnessParts};
pub use decision_records::{DecisionPayload, DecisionRecord, DecisionRecordKind, EvidenceStatus};
pub use episode::{
    ActionAdmission, ActionIdentity, ActionKind, ActionLedger, ActionSetError, BarrierError,
    BarrierPort, CoopCoordinator, CoopError, CoopPeerRole, CoopSyncStatus, DecisionInput,
    DecisionSource, DispatchStatus, EpisodeActionPort, EpisodeCleanupReport, EpisodeLegalAction,
    EpisodeLegalActionSet, EpisodeLifecyclePort, EpisodeMachine, EpisodeMachineError,
    EpisodeObservation, EpisodeObservationPort, EpisodePhase, EpisodeRunFailure, EpisodeRunReport,
    EpisodeRunner, EpisodeRunnerConfig, EpisodeRunnerError, EpisodeRuntimePort, EpisodeShutdown,
    EpisodeStage, ExoDecisionSource, IdempotencyError, NoncombatCoordinator, NoncombatStage,
    ObservationError, PolicyChoice, PolicyError, PolicyRouter, PostconditionError,
    ProtectedEpisodePort, ReceiptQueryActionKind, ReceiptQueryCoordinate, ReceiptQueryError,
    ReceiptQueryIdentity, ReceiptQueryIdentityError, ReceiptQueryLocation, ReceiptQueryReceipt,
    ReceiptQueryResult, ReceiptQueryStatus, RecoveryController, RecoveryError, RecoveryOperation,
    RecoveryPort, RecoveryResult, RunSetupCoordinator, SetupPort, ShutdownError, ShutdownPort,
    StabilityBarrier, TransitionReceipt, VerifiedTransition, WaitOutcome, WaitSample,
    verify_settlement,
};
pub use error::{CloseFailure, CloseReport, Component, HarnessError, PortError, ProviderError};
pub use evaluation::{
    EvaluationError, EvaluationReport, EvaluationSample, Evaluator, TerminalOutcome,
};
pub use exact_transition::{
    AncestrySplit, BranchPolicy, Experiment, ExperimentBranch, ExperimentError, LineageError,
    MAX_BRANCHES, MAX_OCCURRENCES, MAX_TRANSITION_LABEL_BYTES, MAX_TRANSITION_RECORDS,
    OccurrenceGraph, OccurrenceId, OccurrenceRecord, TRANSITION_COMMITMENT_PREFIX,
    TRANSITION_COMMITMENT_VERSION, TRANSITION_DOMAIN, TraceComparison, TraceOutcome,
    TransitionError, TransitionRecord, TransitionTrace, compare_traces, split_by_ancestry,
};
pub use execution::{
    AttemptKind, AttemptState, BLOB_DIGEST_PREFIX, BlobDigest, CHECKPOINT_MANIFEST_DOMAIN,
    CatalogEvidence, Checkpoint, CompletionRecord, CompletionStatus, DecisionReference,
    EXACT_CHECKPOINT_ID_PREFIX, EXACT_STATE_DIGEST_PREFIX, ExactArtifactStore, ExactAssurance,
    ExactCheckpointError, ExactCheckpointId, ExactCheckpointReference, ExactRetentionPlan,
    ExactStateDigest, ExecutionFingerprint, ExecutionLineage, ExecutionStore, ExecutionStoreConfig,
    ExecutionStoreError, GameOperationId, InvocationOutcome, InvocationState, JobClaim,
    JobClaimOutcome, JobState, MAX_BOUNDARY_LABEL_BYTES, MAX_CATALOG_BYTES, MAX_EXACT_BLOB_BYTES,
    MAX_OPERATION_ACTION_BYTES, MAX_ORIGINAL_CONTEXT_BYTES, MAX_WORKFLOW_BYTES,
    MAX_WORKFLOW_COUNTER_NAME_BYTES, MAX_WORKFLOW_COUNTERS, MAX_WORKFLOW_CURSOR,
    MAX_WORKFLOW_STACK_DEPTH, OperationIntent, OperationState, ProviderFailureClass,
    ProviderReservation, ProviderReservationState, RECOVERY_CONTRACT_VERSION,
    RECOVERY_SCHEMA_DIGEST, RecoveryDisposition, ResumeState, RetentionError, RunProjection,
    RunStatus, StorePragmas, StoredAttempt, StoredDecision, StoredEpisode, StoredJob,
    StoredOperation, StoredWorkerHandoff, StoredWorkflowInvocation, WORKER_EMPTY_PARAMETERS_DIGEST,
    WORKER_HANDOFF_CONTRACT, WORKER_HANDOFF_SCHEMA_DIGEST, WORKER_MAX_ATTEMPT_NUMBER,
    WORKFLOW_CONTRACT_VERSION, WorkerAdmissionContext, WorkerAdmissionOutcome, WorkerBoot,
    WorkerCompletionStatus, WorkerControlMode, WorkerControlRequest, WorkerControlState,
    WorkerExecutionPermit, WorkerHandoffState, WorkerLookup, WorkerOwnerProof,
    WorkerReservationState, WorkerTerminalReceipt, WorkerTuple, WorkflowCommandId,
    WorkflowDefinition, WorkflowDefinitionId, WorkflowEpisodeId, WorkflowEvent, WorkflowEventId,
    WorkflowEventPayload, WorkflowInvocation, WorkflowInvocationId, WorkflowPlan, WorkflowPlanId,
    WorkflowRunId, WorkflowRunSnapshot, WorkflowRunStart, plan_retention, sweep,
};
pub use execution_cancellation::ExecutionCancellation;
pub use exo::{
    BoundDecision, CodexEventAccounting, CodexEventError, CodexStreamStatus, CodexTokenUsage,
    CodexUsageStatus, Decision, DecisionError, EXO_MAP_REQUEST_OVERHEAD_BYTES,
    EXO_MAX_MAP_REQUEST_BYTES, EXO_MAX_STANDARD_REQUEST_BYTES, ExoClient, ExoConfig,
    ExoDecisionRequest, ExoError, ExoProvider, ExoSession, ExoTransport, ExoTransportError,
    SandboxError, SanitizedObservation, parse_codex_events, parse_decision,
};
pub use exo_process::{ExoProcessConfig, ExoProcessConfigError, ExoProcessTransport};
pub use identity::{
    ActionId, ArtifactId, Digest, EpisodeId, GatewaySessionId, IdempotencyKey, InstanceId,
    ModelExecutionId, RecordId, RequestId, RunId, SchemaVersion, TraceId, TrajectoryId,
};
pub use management::*;
pub use map::{
    AnalysisCacheKey, AnalysisConfig, ApproximationStatus, BoundedCache, BundleContents,
    BundleFileStore, BundleHistory, BundleManifest, BundleOrigin, BundlePresentation, CacheError,
    CandidateRoute, ContextMode, CountStatus, HistoricalActionBinding, HistoricalReplay,
    InFlightGuard, LegalDestination, LegalDestinationPathCount, MAP_ANALYSIS_MAX_CANDIDATES,
    MAP_ANALYSIS_VERSION, MAP_BUNDLE_VERSION, MAP_CACHE_MAX_BYTES, MAP_CACHE_MAX_ENTRIES,
    MAP_CACHE_MAX_IN_FLIGHT, MAP_FEED_FILE, MAP_FEED_VERSION, MAP_MAX_ACTION_ID_BYTES,
    MAP_MAX_BUNDLE_BYTES, MAP_MAX_CATEGORY_BYTES, MAP_MAX_EDGES, MAP_MAX_FEED_ENTRIES,
    MAP_MAX_IDENTIFIER_BYTES, MAP_MAX_NODES, MAP_MAX_PNG_BYTES, MAP_MAX_PRESENTATION_HEIGHT,
    MAP_MAX_PRESENTATION_PIXELS, MAP_MAX_PRESENTATION_WIDTH, MAP_MAX_SNAPSHOT_BYTES,
    MAP_MIN_PRESENTATION_HEIGHT, MAP_MIN_PRESENTATION_WIDTH, MapAnalysis, MapAnalysisError,
    MapBundleError, MapBundleFeed, MapCompleteness, MapEdge, MapEvaluationError, MapFeed,
    MapFeedEntry, MapGraphError, MapNode, MapNodeStatus, MapViewBundle, NavigationCacheKey,
    NodeMetrics, PublicationReceipt, RUNTIME_MAP_SCHEMA_DIGEST, RUNTIME_MAP_UNRENDERED_DECISION,
    RenderCacheKey, RoutePolicy, RouteScore, RuntimeMapBundleIdentity,
    SYNTHETIC_MAX_ACTION_ID_BYTES, SYNTHETIC_MAX_DECISIONS, SYNTHETIC_MAX_EDGES,
    SYNTHETIC_MAX_IDENTIFIER_BYTES, SYNTHETIC_MAX_LATENCY_MICROS, SYNTHETIC_MAX_NODES,
    SYNTHETIC_MAX_REQUEST_BYTES, SYNTHETIC_MAX_ROUTE_NODES, SYNTHETIC_MAX_ROUTES,
    SYNTHETIC_MAX_TASKS, SyntheticDecision, SyntheticEvaluationReport, SyntheticEvaluationRow,
    SyntheticEvaluationRunner, SyntheticGraphTask, TerminalPathCount, TopologyCacheKey,
    TopologySummary, ValidatedMapGraph, build_unrendered_runtime_map_bundle,
    run_bounded_synthetic_context_matrix, synthetic_action_id, synthetic_context_bytes,
};
pub use memory::{DecisionMemory, MemoryAppend, MemoryError};
pub use poc::{
    POC_CLOCK_TICK, POC_SEED, PocAction, PocCoreError, PocError, PocObservation, PocReport,
    PocRunner, PocStatus, TraceEvent, run_poc,
};
pub use protocol_artifact::{
    ArtifactError, POC_ARTIFACT, POC_GENERATOR, POC_MAX_SETTLED_EFFECTS, POC_MAX_UNITS,
    POC_PROTOCOL_VERSION, POC_SCHEMA_DIGEST, POC_SCHEMA_SOURCE, verify_poc_artifact,
};
pub use protocol_artifact_coop_receipt_query::{
    COOP_RECEIPT_QUERY_ARTIFACT, COOP_RECEIPT_QUERY_GENERATOR, COOP_RECEIPT_QUERY_PROTOCOL_VERSION,
    COOP_RECEIPT_QUERY_SCHEMA_DIGEST, COOP_RECEIPT_QUERY_SCHEMA_SOURCE,
    CoopReceiptQueryArtifactError, verify_coop_receipt_query_artifact,
};
pub use provider::{
    ModelOutput, ModelRequest, ModelResponse, ModelResult, Prompt, ProviderPort, RetryPolicy,
};
pub use records::{AppendOutcome, Correlation, Record, RecordKind, RecordPayload, RecordPort};
pub use replay::{
    DecisionReplay, DecisionReplayDivergence, DecisionReplayReport, DecisionReplayRequest,
    DeterministicReplay, Divergence, ReplayPort, ReplayReport, ReplayRequest,
};
pub use restore_gate::{GateAdmission, GateError, RestoreEvidence, RestoreGate, RestoreReceipt};
pub use routing::{InstanceRouter, RouteBinding, RouteRequest, RouteToken};
pub use runtime_v2::{
    RUNTIME_V2_MAX_INSTANCES, RUNTIME_V2_MAX_QUEUE_CAPACITY, RUNTIME_V2_MAX_RETAINED_OPERATIONS,
    RuntimeV2Action, RuntimeV2ArtifactLineage, RuntimeV2ArtifactRecord, RuntimeV2CombatPhase,
    RuntimeV2Context, RuntimeV2Coordinator, RuntimeV2CoordinatorConfig, RuntimeV2CoordinatorError,
    RuntimeV2CoordinatorSnapshot, RuntimeV2EffectWitness, RuntimeV2Error, RuntimeV2EventKind,
    RuntimeV2Evidence, RuntimeV2InstanceBinding, RuntimeV2InstanceSnapshot, RuntimeV2Kind,
    RuntimeV2Message, RuntimeV2NoRetryEvidence, RuntimeV2Observation, RuntimeV2OperationId,
    RuntimeV2Provenance, RuntimeV2Record, RuntimeV2RecordKind, RuntimeV2Report, RuntimeV2Runner,
    RuntimeV2ShutdownReport, RuntimeV2Status, RuntimeV2Trajectory, RuntimeV2WorkItem,
    run_runtime_v2_fake_trace,
};
pub use runtime_v2_artifact::{
    RUNTIME_V2_ARTIFACT, RUNTIME_V2_GENERATOR, RUNTIME_V2_MAX_GENERATION,
    RUNTIME_V2_MAX_IDENTITY_BYTES, RUNTIME_V2_MAX_LEASE_EPOCH, RUNTIME_V2_MAX_RECORDS,
    RUNTIME_V2_MAX_TURN_INDEX, RUNTIME_V2_PROTOCOL_VERSION, RUNTIME_V2_SCHEMA_DIGEST,
    RUNTIME_V2_SCHEMA_SOURCE, RuntimeV2ArtifactError, runtime_v2_manifest_bytes,
    runtime_v2_schema_bytes, verify_runtime_v2_artifact,
};
pub use runtime_v4_expert::{RuntimeV4ExpertObservation, RuntimeV4ExpertParseError};
pub use runtime_v4_expert_action::{
    RuntimeV4ExpertActionParseError, RuntimeV4ExpertActionRequest, RuntimeV4ExpertActionResult,
    RuntimeV4ExpertActionStatus,
};
pub use runtime_v4_expert_action_artifact::{
    RUNTIME_V4_EXPERT_ACTION_ARTIFACT, RUNTIME_V4_EXPERT_ACTION_GENERATOR,
    RUNTIME_V4_EXPERT_ACTION_PROTOCOL_VERSION, RUNTIME_V4_EXPERT_ACTION_SCHEMA_DIGEST,
    RUNTIME_V4_EXPERT_ACTION_SCHEMA_SOURCE, RuntimeV4ExpertActionArtifactError,
    verify_runtime_v4_expert_action_artifact,
};
pub use runtime_v4_expert_artifact::{
    RUNTIME_V4_EXPERT_ARTIFACT, RUNTIME_V4_EXPERT_FAIR_PLAY_PROJECTION,
    RUNTIME_V4_EXPERT_GENERATOR, RUNTIME_V4_EXPERT_PROTOCOL_VERSION,
    RUNTIME_V4_EXPERT_SCHEMA_DIGEST, RUNTIME_V4_EXPERT_SCHEMA_SOURCE, RuntimeV4ExpertArtifactError,
    verify_runtime_v4_expert_artifact,
};
pub use runtime_v4_expert_rest_action::{
    RuntimeV4ExpertRestActionParseError, RuntimeV4ExpertRestActionRequest,
    RuntimeV4ExpertRestActionResult, RuntimeV4ExpertRestActionStatus,
};
pub use runtime_v4_expert_rest_action_artifact::{
    RUNTIME_V4_EXPERT_REST_ACTION_ARTIFACT, RUNTIME_V4_EXPERT_REST_ACTION_GENERATOR,
    RUNTIME_V4_EXPERT_REST_ACTION_PROTOCOL_VERSION, RUNTIME_V4_EXPERT_REST_ACTION_SCHEMA_DIGEST,
    RUNTIME_V4_EXPERT_REST_ACTION_SCHEMA_SOURCE, RuntimeV4ExpertRestActionArtifactError,
    verify_runtime_v4_expert_rest_action_artifact,
};
