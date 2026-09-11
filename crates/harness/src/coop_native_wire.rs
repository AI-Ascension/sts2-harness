// SPDX-License-Identifier: MIT

use super::artifact::{
    COOP_NATIVE_ARTIFACT, COOP_NATIVE_GENERATOR, COOP_NATIVE_MAX_REQUEST_BYTES,
    COOP_NATIVE_MAX_RESPONSE_BYTES, COOP_NATIVE_PROTOCOL_VERSION, COOP_NATIVE_SCHEMA_DIGEST,
    COOP_NATIVE_SCHEMA_SOURCE,
};
use super::identity::{
    CoopNativeActionId, CoopNativeIdentityError, CoopNativeLineage,
    CoopNativeOperationId, CoopNativePeerId,
};
use serde_json::Value;

mod types {
    include!("coop_native_wire_types.rs");
}
pub use types::*;

pub const COOP_NATIVE_MAX_DEPTH: usize = 16;

pub(crate) const ROOT_FIELDS: [&str; 20] = [
    "protocol_version",
    "schema_digest",
    "provenance",
    "correlation_id",
    "instance_id",
    "session_id",
    "lease_id",
    "lease_epoch",
    "kind",
    "operation_id",
    "actor_peer",
    "expected_host_generation",
    "action",
    "vote",
    "status",
    "observation",
    "effect",
    "recovery",
    "catalog",
    "receipt",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoopNativeEnvelopeError {
    TooLarge,
    MalformedJson,
    DuplicateMember,
    DepthExceeded,
    InvalidShape,
    InvalidValue,
    UnsupportedKind,
    SchemaDigestMismatch,
    ArtifactMismatch,
    WrongDirection,
    IdentityMismatch,
}

impl std::fmt::Display for CoopNativeEnvelopeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::TooLarge => "native co-op envelope exceeds its byte bound",
            Self::MalformedJson => "native co-op envelope is malformed JSON",
            Self::DuplicateMember => "native co-op envelope contains a duplicate member",
            Self::DepthExceeded => "native co-op envelope exceeds its depth bound",
            Self::InvalidShape => "native co-op envelope has an invalid closed shape",
            Self::InvalidValue => "native co-op envelope has an invalid value",
            Self::UnsupportedKind => "native co-op envelope kind is unsupported",
            Self::SchemaDigestMismatch => "native co-op envelope schema digest is not admitted",
            Self::ArtifactMismatch => "native co-op candidate artifact verification failed",
            Self::WrongDirection => "native co-op envelope has the wrong request/response direction",
            Self::IdentityMismatch => "native co-op envelope identity does not match its lineage",
        })
    }
}

impl std::error::Error for CoopNativeEnvelopeError {}

impl From<CoopNativeIdentityError> for CoopNativeEnvelopeError {
    fn from(_: CoopNativeIdentityError) -> Self {
        Self::InvalidValue
    }
}

/// A strictly validated native observation/action/vote/rejoin/effect/recovery envelope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoopNativeEnvelope {
    header: CoopNativeHeader,
    kind: CoopNativeKind,
    body: CoopNativeBody,
    value: Value,
}

impl CoopNativeEnvelope {
    #[must_use]
    pub const fn kind(&self) -> CoopNativeKind {
        self.kind
    }

    #[must_use]
    pub fn header(&self) -> &CoopNativeHeader {
        &self.header
    }

    #[must_use]
    pub fn operation_id(&self) -> Option<&CoopNativeOperationId> {
        match &self.body {
            CoopNativeBody::Observation(_) => None,
            CoopNativeBody::LegalCatalogRequest(_) | CoopNativeBody::LegalCatalogResponse(_) => {
                None
            }
            CoopNativeBody::LocalActionRequest(request) => Some(&request.operation_id),
            CoopNativeBody::SharedVoteRequest(request) => Some(&request.operation_id),
            CoopNativeBody::RejoinRequest(request) => Some(&request.operation_id),
            CoopNativeBody::EffectResponse(response) => Some(&response.operation_id),
            CoopNativeBody::RecoveryResponse(response) => Some(&response.operation_id),
        }
    }

    /// Returns the generation fenced by a request, or by response receipt evidence.
    #[must_use]
    pub fn expected_host_generation(&self) -> Option<u64> {
        match &self.body {
            CoopNativeBody::LegalCatalogRequest(request) => {
                Some(request.expected_host_generation)
            }
            CoopNativeBody::LegalCatalogResponse(response) => {
                Some(response.expected_host_generation)
            }
            CoopNativeBody::LocalActionRequest(request) => Some(request.expected_host_generation),
            CoopNativeBody::SharedVoteRequest(request) => Some(request.expected_host_generation),
            CoopNativeBody::RejoinRequest(request) => Some(request.expected_host_generation),
            CoopNativeBody::EffectResponse(response) => {
                Some(response.receipt.before_host_generation)
            }
            CoopNativeBody::RecoveryResponse(response) => response
                .receipt
                .as_ref()
                .map(|receipt| receipt.before_host_generation),
            CoopNativeBody::Observation(_) => None,
        }
    }

    /// Returns receipt evidence carried by a mutation or recovery response.
    #[must_use]
    pub fn receipt(&self) -> Option<&CoopNativeReceipt> {
        match &self.body {
            CoopNativeBody::EffectResponse(response) => Some(&response.receipt),
            CoopNativeBody::RecoveryResponse(response) => response.receipt.as_ref(),
            _ => None,
        }
    }

    #[must_use]
    pub fn action_id(&self) -> Option<&CoopNativeActionId> {
        match &self.body {
            CoopNativeBody::LocalActionRequest(request) => Some(&request.action.action_id),
            _ => None,
        }
    }

    #[must_use]
    pub fn observation(&self) -> Option<&CoopNativeObservation> {
        match &self.body {
            CoopNativeBody::Observation(observation) => Some(observation),
            CoopNativeBody::LegalCatalogResponse(response) => Some(&response.observation),
            CoopNativeBody::EffectResponse(response) => Some(&response.observation),
            CoopNativeBody::RecoveryResponse(response) => response.observation.as_ref(),
            _ => None,
        }
    }

    /// Returns the actor that names an observation's canonical peer when present.
    #[must_use]
    pub fn actor_peer(&self) -> Option<&CoopNativePeerId> {
        match &self.body {
            CoopNativeBody::LegalCatalogRequest(request) => Some(&request.actor_peer),
            CoopNativeBody::LegalCatalogResponse(response) => Some(&response.actor_peer),
            CoopNativeBody::LocalActionRequest(request) => Some(&request.actor_peer),
            CoopNativeBody::SharedVoteRequest(request) => Some(&request.actor_peer),
            CoopNativeBody::RejoinRequest(request) => Some(&request.actor_peer),
            CoopNativeBody::Observation(_)
            | CoopNativeBody::EffectResponse(_)
            | CoopNativeBody::RecoveryResponse(_) => None,
        }
    }

    #[must_use]
    pub fn action_request(&self) -> Option<&CoopNativeLocalActionRequest> {
        match &self.body {
            CoopNativeBody::LocalActionRequest(request) => Some(request),
            _ => None,
        }
    }

    #[must_use]
    pub fn legal_catalog_request(&self) -> Option<&CoopNativeLegalCatalogRequest> {
        match &self.body {
            CoopNativeBody::LegalCatalogRequest(request) => Some(request),
            _ => None,
        }
    }

    #[must_use]
    pub fn legal_catalog_response(&self) -> Option<&CoopNativeLegalCatalogResponse> {
        match &self.body {
            CoopNativeBody::LegalCatalogResponse(response) => Some(response),
            _ => None,
        }
    }

    #[must_use]
    pub fn vote_request(&self) -> Option<&CoopNativeSharedVoteRequest> {
        match &self.body {
            CoopNativeBody::SharedVoteRequest(request) => Some(request),
            _ => None,
        }
    }

    #[must_use]
    pub fn rejoin_request(&self) -> Option<&CoopNativeRejoinRequest> {
        match &self.body {
            CoopNativeBody::RejoinRequest(request) => Some(request),
            _ => None,
        }
    }

    #[must_use]
    pub fn effect_response(&self) -> Option<&CoopNativeEffectResponse> {
        match &self.body {
            CoopNativeBody::EffectResponse(response) => Some(response),
            _ => None,
        }
    }

    #[must_use]
    pub fn recovery_response(&self) -> Option<&CoopNativeRecoveryResponse> {
        match &self.body {
            CoopNativeBody::RecoveryResponse(response) => Some(response),
            _ => None,
        }
    }

    /// Returns the parsed value retained for semantic replay comparisons.
    #[must_use]
    pub fn as_value(&self) -> &Value {
        &self.value
    }

    /// Checks independent native identities against the harness lineage.
    pub fn validate_lineage(&self, lineage: &CoopNativeLineage) -> Result<(), CoopNativeEnvelopeError> {
        if self.header.instance_id.as_str() != lineage.instance_id().as_str()
            || self.header.session_id.as_str() != lineage.session_id().as_str()
        {
            return Err(CoopNativeEnvelopeError::IdentityMismatch);
        }
        if let Some(expected) = lineage.operation_id()
            && self.operation_id() != Some(expected)
        {
            return Err(CoopNativeEnvelopeError::IdentityMismatch);
        }
        if let Some(expected) = lineage.action_id()
            && self.action_id() != Some(expected)
        {
            return Err(CoopNativeEnvelopeError::IdentityMismatch);
        }
        if let Some(observation) = self.observation()
            && observation.run_id() != lineage.run_id()
        {
            return Err(CoopNativeEnvelopeError::IdentityMismatch);
        }
        Ok(())
    }
}

include!("coop_native_wire_validation.rs");
