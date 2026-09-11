// SPDX-License-Identifier: MIT

mod identity {
    include!("coop_native_identity.rs");
}

mod artifact {
    include!("coop_native_artifact.rs");
}

mod wire {
    include!("coop_native_wire.rs");
}

mod port {
    include!("coop_native_port.rs");
}

mod coordinator {
    include!("coop_native_coordinator.rs");
}

mod cohort {
    include!("coop_native_cohort.rs");
}

pub use artifact::{
    COOP_NATIVE_ARTIFACT, COOP_NATIVE_GENERATOR, COOP_NATIVE_MAX_RECORDS,
    COOP_NATIVE_MAX_REQUEST_BYTES, COOP_NATIVE_MAX_RESPONSE_BYTES,
    COOP_NATIVE_PRODUCER_SCHEMA_DIGEST, COOP_NATIVE_PROTOCOL_VERSION, COOP_NATIVE_SCHEMA_DIGEST,
    COOP_NATIVE_SCHEMA_SOURCE, CoopNativeAdmissionStatus, CoopNativeArtifactError,
    CoopNativeArtifactLineage, CoopNativeArtifactRecord, CoopNativeArtifactState,
    CoopNativeArtifactStatus, coop_native_manifest_bytes, coop_native_schema_bytes,
    verify_coop_native_artifact, verify_coop_native_candidate_artifact,
};
pub use cohort::{
    CoopNativeCohort, CoopNativeCohortError, CoopNativeCohortOperation, CoopNativeCohortRoute,
};
pub use coordinator::{
    CoopNativeCoordinator, CoopNativeCoordinatorError, CoopNativeEventKind,
    CoopNativeOperationState, CoopNativeReceipt, CoopNativeRecord, CoopNativeReplayReport,
    replay_coop_native_records,
};
pub use identity::{
    CoopNativeActionId, CoopNativeArtifactId, CoopNativeAuthorityId, CoopNativeCheckpointId,
    CoopNativeCorrelationId, CoopNativeEffectId, CoopNativeEpisodeId, CoopNativeIdentityError,
    CoopNativeInstanceId, CoopNativeLeaseId, CoopNativeLineage, CoopNativeModelExecutionId,
    CoopNativeOperationId, CoopNativePeerId, CoopNativeProposalId, CoopNativeRequestId,
    CoopNativeRunId, CoopNativeSessionId, CoopNativeTraceId, CoopNativeTrajectoryId,
};
pub use port::{CoopNativePort, CoopNativePortError};
pub use wire::{
    COOP_NATIVE_MAX_DEPTH, CoopNativeAction, CoopNativeActionKind, CoopNativeChecksumStatus,
    CoopNativeEffect, CoopNativeEffectKind, CoopNativeEffectResponse, CoopNativeEnvelope,
    CoopNativeEnvelopeError, CoopNativeHeader, CoopNativeKind, CoopNativeLocalActionRequest,
    CoopNativeObservation, CoopNativePeerRole, CoopNativePeerSnapshot, CoopNativeRecovery,
    CoopNativeRecoveryKind, CoopNativeRecoveryResponse, CoopNativeRejoinRequest,
    CoopNativeSharedVoteRequest, CoopNativeStatus, CoopNativeVote,
};

#[cfg(test)]
mod tests {
    include!("coop_native_tests.rs");
}
