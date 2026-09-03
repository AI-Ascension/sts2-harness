// SPDX-License-Identifier: MIT

mod artifact;
mod coordinator;
mod error;
mod identity;
mod poc;
mod protocol_artifact;
mod provider;
mod records;
mod replay;
mod routing;
mod runtime_v2;
mod runtime_v2_artifact;

pub use artifact::{
    ArtifactDraft, ArtifactKind, ArtifactLineage, ArtifactMetadata, ArtifactMetadataInput,
    ArtifactPort, ArtifactPublicationRequest, ArtifactReceipt,
};
pub use coordinator::{EpisodeHandle, Harness, HarnessParts};
pub use error::{CloseFailure, CloseReport, Component, HarnessError, PortError, ProviderError};
pub use identity::{
    ActionId, ArtifactId, Digest, EpisodeId, GatewaySessionId, IdempotencyKey, InstanceId,
    ModelExecutionId, RecordId, RequestId, RunId, SchemaVersion, TraceId, TrajectoryId,
};
pub use poc::{
    POC_CLOCK_TICK, POC_SEED, PocAction, PocCoreError, PocError, PocObservation, PocReport,
    PocRunner, PocStatus, TraceEvent, run_poc,
};
pub use protocol_artifact::{
    ArtifactError, POC_ARTIFACT, POC_GENERATOR, POC_MAX_SETTLED_EFFECTS, POC_MAX_UNITS,
    POC_PROTOCOL_VERSION, POC_SCHEMA_DIGEST, POC_SCHEMA_SOURCE, verify_poc_artifact,
};
pub use provider::{
    ModelOutput, ModelRequest, ModelResponse, ModelResult, Prompt, ProviderPort, RetryPolicy,
};
pub use records::{AppendOutcome, Correlation, Record, RecordKind, RecordPayload, RecordPort};
pub use replay::{DeterministicReplay, Divergence, ReplayPort, ReplayReport, ReplayRequest};
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
