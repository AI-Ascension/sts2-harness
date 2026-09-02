// SPDX-License-Identifier: MIT

use super::contract::{PocAction, PocError, PocObservation, PocStatus};
use super::wire::{
    PocWireAction, PocWireKind, PocWireMessage, PocWireProvenance, canonical_wire_json,
};
use crate::protocol_artifact::{POC_PROTOCOL_VERSION, POC_SCHEMA_DIGEST};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PocResponse {
    correlation_id: String,
    instance_id: String,
    generation: u64,
    observation: PocObservation,
    action: Option<PocAction>,
    status: Option<PocStatus>,
    error_code: Option<&'static str>,
}

impl PocResponse {
    pub(super) fn state(
        correlation_id: &str,
        instance_id: &str,
        generation: u64,
        observation: PocObservation,
    ) -> Self {
        Self {
            correlation_id: correlation_id.to_owned(),
            instance_id: instance_id.to_owned(),
            generation,
            observation,
            action: None,
            status: None,
            error_code: None,
        }
    }

    pub(super) fn action_response(
        correlation_id: &str,
        instance_id: &str,
        generation: u64,
        observation: PocObservation,
        action: PocAction,
        status: PocStatus,
        error_code: Option<&'static str>,
    ) -> Self {
        Self {
            correlation_id: correlation_id.to_owned(),
            instance_id: instance_id.to_owned(),
            generation,
            observation,
            action: Some(action),
            status: Some(status),
            error_code,
        }
    }

    pub(super) const fn observation(&self) -> PocObservation {
        self.observation
    }

    pub(super) const fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn correlation_id(&self) -> &str {
        &self.correlation_id
    }

    pub(super) fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub(super) const fn action(&self) -> Option<PocAction> {
        self.action
    }

    pub(super) const fn status(&self) -> Option<PocStatus> {
        self.status
    }

    pub(super) const fn error_code(&self) -> Option<&'static str> {
        self.error_code
    }

    pub(super) fn kind(&self) -> &'static str {
        if self.action.is_some() {
            PocWireKind::ActionResponse.as_str()
        } else {
            PocWireKind::StateResponse.as_str()
        }
    }

    pub(super) fn wire_json(&self) -> Result<String, PocError> {
        canonical_wire_json(&self.wire_message())
            .map_err(|_| PocError::InvalidTrace("POC response does not match the wire contract"))
    }

    fn wire_message(&self) -> PocWireMessage {
        PocWireMessage {
            protocol_version: POC_PROTOCOL_VERSION.to_owned(),
            schema_digest: POC_SCHEMA_DIGEST.to_owned(),
            provenance: PocWireProvenance::default(),
            correlation_id: self.correlation_id.clone(),
            instance_id: self.instance_id.clone(),
            generation: self.generation,
            kind: if self.action.is_some() {
                PocWireKind::ActionResponse
            } else {
                PocWireKind::StateResponse
            },
            observation: Some(self.observation),
            action: self.action.map(PocWireAction::from_action),
            status: self.status,
            error_code: self.error_code.map(str::to_owned),
        }
    }
}
