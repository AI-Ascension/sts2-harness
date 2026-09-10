// SPDX-License-Identifier: MIT

use serde::Serialize;
use std::fmt;

pub(crate) const MAX_IDENTITY_BYTES: usize = 512;
pub(crate) const MAX_GENERATION: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoopNativeIdentityError {
    Empty,
    TooLong,
    InvalidCharacters,
    InvalidPeer,
    InvalidGeneration,
}

impl fmt::Display for CoopNativeIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "native co-op identity is empty",
            Self::TooLong => "native co-op identity exceeds 512 bytes",
            Self::InvalidCharacters => "native co-op identity contains an invalid character",
            Self::InvalidPeer => "native co-op peer identity is invalid",
            Self::InvalidGeneration => "native co-op generation exceeds the JSON-safe bound",
        })
    }
}

impl std::error::Error for CoopNativeIdentityError {}

pub(crate) fn validate_identity(value: &str) -> Result<(), CoopNativeIdentityError> {
    if value.is_empty() {
        return Err(CoopNativeIdentityError::Empty);
    }
    if value.len() > MAX_IDENTITY_BYTES {
        return Err(CoopNativeIdentityError::TooLong);
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
    }) {
        return Err(CoopNativeIdentityError::InvalidCharacters);
    }
    Ok(())
}

pub(crate) fn validate_generation(value: u64) -> Result<(), CoopNativeIdentityError> {
    (value <= MAX_GENERATION)
        .then_some(())
        .ok_or(CoopNativeIdentityError::InvalidGeneration)
}

pub(crate) trait NativeId: Clone + Eq + Ord {
    fn parse(value: String) -> Result<Self, CoopNativeIdentityError>;
}

macro_rules! identity_type {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, CoopNativeIdentityError> {
                let value = value.into();
                validate_identity(&value)?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl NativeId for $name {
            fn parse(value: String) -> Result<Self, CoopNativeIdentityError> {
                Self::new(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

identity_type!(CoopNativeCorrelationId);
identity_type!(CoopNativeInstanceId);
identity_type!(CoopNativeSessionId);
identity_type!(CoopNativeLeaseId);
identity_type!(CoopNativeOperationId);
identity_type!(CoopNativeActionId);
identity_type!(CoopNativeRunId);
identity_type!(CoopNativeEpisodeId);
identity_type!(CoopNativeTrajectoryId);
identity_type!(CoopNativeRequestId);
identity_type!(CoopNativeTraceId);
identity_type!(CoopNativeModelExecutionId);
identity_type!(CoopNativeArtifactId);
identity_type!(CoopNativeAuthorityId);
identity_type!(CoopNativeCheckpointId);
identity_type!(CoopNativeEffectId);
identity_type!(CoopNativeProposalId);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct CoopNativePeerId(String);

impl CoopNativePeerId {
    pub fn new(value: impl Into<String>) -> Result<Self, CoopNativeIdentityError> {
        let value = value.into();
        validate_identity(&value)?;
        if value.len() < 10 || !value.starts_with("peer:") {
            return Err(CoopNativeIdentityError::InvalidPeer);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl NativeId for CoopNativePeerId {
    fn parse(value: String) -> Result<Self, CoopNativeIdentityError> {
        Self::new(value)
    }
}

impl fmt::Display for CoopNativePeerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Harness-owned identities that surround a native envelope without collapsing namespaces.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CoopNativeLineage {
    instance_id: CoopNativeInstanceId,
    session_id: CoopNativeSessionId,
    run_id: CoopNativeRunId,
    episode_id: CoopNativeEpisodeId,
    trajectory_id: CoopNativeTrajectoryId,
    request_id: CoopNativeRequestId,
    action_id: Option<CoopNativeActionId>,
    operation_id: Option<CoopNativeOperationId>,
    trace_id: CoopNativeTraceId,
    model_execution_id: Option<CoopNativeModelExecutionId>,
    artifact_id: CoopNativeArtifactId,
}

impl CoopNativeLineage {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        instance_id: CoopNativeInstanceId,
        session_id: CoopNativeSessionId,
        run_id: CoopNativeRunId,
        episode_id: CoopNativeEpisodeId,
        trajectory_id: CoopNativeTrajectoryId,
        request_id: CoopNativeRequestId,
        action_id: Option<CoopNativeActionId>,
        operation_id: Option<CoopNativeOperationId>,
        trace_id: CoopNativeTraceId,
        model_execution_id: Option<CoopNativeModelExecutionId>,
        artifact_id: CoopNativeArtifactId,
    ) -> Self {
        Self {
            instance_id,
            session_id,
            run_id,
            episode_id,
            trajectory_id,
            request_id,
            action_id,
            operation_id,
            trace_id,
            model_execution_id,
            artifact_id,
        }
    }

    #[must_use]
    pub fn instance_id(&self) -> &CoopNativeInstanceId {
        &self.instance_id
    }

    #[must_use]
    pub fn session_id(&self) -> &CoopNativeSessionId {
        &self.session_id
    }

    #[must_use]
    pub fn run_id(&self) -> &CoopNativeRunId {
        &self.run_id
    }

    #[must_use]
    pub fn episode_id(&self) -> &CoopNativeEpisodeId {
        &self.episode_id
    }

    #[must_use]
    pub fn trajectory_id(&self) -> &CoopNativeTrajectoryId {
        &self.trajectory_id
    }

    #[must_use]
    pub fn request_id(&self) -> &CoopNativeRequestId {
        &self.request_id
    }

    #[must_use]
    pub fn action_id(&self) -> Option<&CoopNativeActionId> {
        self.action_id.as_ref()
    }

    #[must_use]
    pub fn operation_id(&self) -> Option<&CoopNativeOperationId> {
        self.operation_id.as_ref()
    }

    #[must_use]
    pub fn trace_id(&self) -> &CoopNativeTraceId {
        &self.trace_id
    }

    #[must_use]
    pub fn model_execution_id(&self) -> Option<&CoopNativeModelExecutionId> {
        self.model_execution_id.as_ref()
    }

    #[must_use]
    pub fn artifact_id(&self) -> &CoopNativeArtifactId {
        &self.artifact_id
    }
}
