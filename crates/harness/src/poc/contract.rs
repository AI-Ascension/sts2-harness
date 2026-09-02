// SPDX-License-Identifier: MIT

use super::wire::{
    PocWireAction, PocWireKind, PocWireMessage, PocWireProvenance, canonical_wire_json,
};
use crate::protocol_artifact::{
    ArtifactError, POC_ARTIFACT, POC_GENERATOR, POC_PROTOCOL_VERSION, POC_SCHEMA_DIGEST,
    POC_SCHEMA_SOURCE,
};
use serde::{Deserialize, Serialize};

/// The typed action exposed by the fake POC.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PocAction {
    action_id: &'static str,
    units: u16,
}

impl PocAction {
    pub(super) const fn new(action_id: &'static str, units: u16) -> Self {
        Self { action_id, units }
    }

    /// Returns the action identity.
    #[must_use]
    pub const fn action_id(self) -> &'static str {
        self.action_id
    }

    /// Returns the bounded typed argument.
    #[must_use]
    pub const fn units(self) -> u16 {
        self.units
    }
}

/// The bounded state projection carried by every fake response.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PocObservation {
    pub available_units: u16,
    pub settled_effects: u16,
}

/// The two result statuses in the fake POC.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PocStatus {
    Accepted,
    Rejected,
}

impl PocStatus {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PocRoute {
    State,
    Action,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PocRequest {
    route: PocRoute,
    correlation_id: String,
    instance_id: String,
    session_id: String,
    generation: u64,
    action: Option<PocAction>,
    lease_id: String,
}

impl PocRequest {
    pub(super) fn state(
        correlation_id: &str,
        instance_id: &str,
        session_id: &str,
        lease_id: &str,
    ) -> Self {
        Self {
            route: PocRoute::State,
            correlation_id: correlation_id.to_owned(),
            instance_id: instance_id.to_owned(),
            session_id: session_id.to_owned(),
            generation: 0,
            action: None,
            lease_id: lease_id.to_owned(),
        }
    }

    pub(super) fn action_request(
        correlation_id: &str,
        instance_id: &str,
        session_id: &str,
        generation: u64,
        action: PocAction,
        lease_id: &str,
    ) -> Self {
        Self {
            route: PocRoute::Action,
            correlation_id: correlation_id.to_owned(),
            instance_id: instance_id.to_owned(),
            session_id: session_id.to_owned(),
            generation,
            action: Some(action),
            lease_id: lease_id.to_owned(),
        }
    }

    pub(super) fn is_valid(
        &self,
        expected_instance: &str,
        expected_session: &str,
        expected_lease: &str,
    ) -> bool {
        self.protocol_version() == POC_PROTOCOL_VERSION
            && self.schema_digest() == POC_SCHEMA_DIGEST
            && self.artifact() == POC_ARTIFACT
            && self.source() == POC_SCHEMA_SOURCE
            && self.generator() == POC_GENERATOR
            && self.instance_id == expected_instance
            && self.session_id == expected_session
            && self.lease_id == expected_lease
            && !self.correlation_id.is_empty()
            && match self.route {
                PocRoute::State => self.action.is_none(),
                PocRoute::Action => self.action.is_some(),
            }
    }

    pub(super) const fn route(&self) -> PocRoute {
        self.route
    }

    pub(super) fn correlation_id(&self) -> &str {
        &self.correlation_id
    }

    pub(super) fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub(super) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(super) fn lease_id(&self) -> &str {
        &self.lease_id
    }

    pub(super) const fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn action(&self) -> Option<PocAction> {
        self.action
    }

    pub(super) const fn protocol_version(&self) -> &'static str {
        POC_PROTOCOL_VERSION
    }

    pub(super) const fn schema_digest(&self) -> &'static str {
        POC_SCHEMA_DIGEST
    }

    pub(super) const fn artifact(&self) -> &'static str {
        POC_ARTIFACT
    }

    pub(super) const fn source(&self) -> &'static str {
        POC_SCHEMA_SOURCE
    }

    pub(super) const fn generator(&self) -> &'static str {
        POC_GENERATOR
    }

    pub(super) fn wire_json(&self) -> Result<String, PocError> {
        canonical_wire_json(&self.wire_message())
            .map_err(|_| PocError::InvalidRequest("POC request does not match the wire contract"))
    }

    fn wire_message(&self) -> PocWireMessage {
        PocWireMessage {
            protocol_version: POC_PROTOCOL_VERSION.to_owned(),
            schema_digest: POC_SCHEMA_DIGEST.to_owned(),
            provenance: PocWireProvenance::default(),
            correlation_id: self.correlation_id.clone(),
            instance_id: self.instance_id.clone(),
            generation: self.generation,
            kind: match self.route {
                PocRoute::State => PocWireKind::StateRequest,
                PocRoute::Action => PocWireKind::ActionRequest,
            },
            observation: None,
            action: self.action.map(PocWireAction::from_action),
            status: None,
            error_code: None,
        }
    }
}

/// Errors from the deterministic fake POC path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PocError {
    Artifact(ArtifactError),
    InvalidRequest(&'static str),
    GatewayFence(&'static str),
    Core(PocCoreError),
    InvalidTrace(&'static str),
}

impl std::fmt::Display for PocError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Artifact(error) => write!(formatter, "artifact error: {error}"),
            Self::InvalidRequest(message) => write!(formatter, "invalid POC request: {message}"),
            Self::GatewayFence(message) => write!(formatter, "gateway fence rejected: {message}"),
            Self::Core(error) => write!(formatter, "core rejected action: {}", error.code()),
            Self::InvalidTrace(message) => write!(formatter, "invalid POC trace: {message}"),
        }
    }
}

impl std::error::Error for PocError {}

/// Core legality failures preserved in the fake response identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PocCoreError {
    StaleGeneration,
    ZeroUnits,
    InsufficientUnits,
}

impl PocCoreError {
    pub(super) const fn code(self) -> &'static str {
        match self {
            Self::StaleGeneration => "sts2.game-core/stale_generation",
            Self::ZeroUnits => "sts2.game-core/zero_units",
            Self::InsufficientUnits => "sts2.game-core/insufficient_units",
        }
    }
}
