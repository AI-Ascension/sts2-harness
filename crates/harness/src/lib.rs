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
