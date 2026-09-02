// SPDX-License-Identifier: MIT

use super::contract::{PocAction, PocObservation, PocStatus};
use crate::protocol_artifact::{
    POC_ARTIFACT, POC_GENERATOR, POC_MAX_GENERATION, POC_MAX_SETTLED_EFFECTS, POC_MAX_UNITS,
    POC_PROTOCOL_VERSION, POC_SCHEMA_DIGEST, POC_SCHEMA_SOURCE,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum PocWireKind {
    StateRequest,
    StateResponse,
    ActionRequest,
    ActionResponse,
}

impl PocWireKind {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::StateRequest => "state_request",
            Self::StateResponse => "state_response",
            Self::ActionRequest => "action_request",
            Self::ActionResponse => "action_response",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PocWireProvenance {
    pub(super) artifact: String,
    pub(super) source: String,
    pub(super) generator: String,
}

impl Default for PocWireProvenance {
    fn default() -> Self {
        Self {
            artifact: POC_ARTIFACT.to_owned(),
            source: POC_SCHEMA_SOURCE.to_owned(),
            generator: POC_GENERATOR.to_owned(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PocWireAction {
    pub(super) action_id: String,
    pub(super) units: u16,
}

impl PocWireAction {
    pub(super) fn from_action(action: PocAction) -> Self {
        Self {
            action_id: action.action_id().to_owned(),
            units: action.units(),
        }
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.action_id != "use_budget" || self.units > POC_MAX_UNITS {
            return Err("wire action is outside the POC bound");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PocWireMessage {
    pub(super) protocol_version: String,
    pub(super) schema_digest: String,
    pub(super) provenance: PocWireProvenance,
    pub(super) correlation_id: String,
    pub(super) instance_id: String,
    pub(super) generation: u64,
    pub(super) kind: PocWireKind,
    pub(super) observation: Option<PocObservation>,
    pub(super) action: Option<PocWireAction>,
    pub(super) status: Option<PocStatus>,
    pub(super) error_code: Option<String>,
}

impl PocWireMessage {
    fn validate(&self) -> Result<(), &'static str> {
        if self.protocol_version != POC_PROTOCOL_VERSION
            || self.schema_digest != POC_SCHEMA_DIGEST
            || self.provenance.artifact != POC_ARTIFACT
            || self.provenance.source != POC_SCHEMA_SOURCE
            || self.provenance.generator != POC_GENERATOR
        {
            return Err("wire metadata does not identify poc-v1");
        }
        validate_identity(&self.correlation_id)?;
        validate_identity(&self.instance_id)?;
        if self.generation > POC_MAX_GENERATION {
            return Err("wire generation is outside the POC bound");
        }
        if let Some(observation) = self.observation
            && (observation.available_units > POC_MAX_UNITS
                || observation.settled_effects > POC_MAX_SETTLED_EFFECTS)
        {
            return Err("wire observation is outside the POC bound");
        }
        if let Some(action) = &self.action {
            action.validate()?;
        }
        if let Some(error_code) = &self.error_code {
            validate_identity(error_code)?;
        }

        let shape = (
            self.observation.is_some(),
            self.action.is_some(),
            self.status,
        );
        match self.kind {
            PocWireKind::StateRequest
                if shape == (false, false, None) && self.error_code.is_none() =>
            {
                Ok(())
            }
            PocWireKind::StateResponse
                if shape == (true, false, None) && self.error_code.is_none() =>
            {
                Ok(())
            }
            PocWireKind::ActionRequest
                if shape == (false, true, None) && self.error_code.is_none() =>
            {
                Ok(())
            }
            PocWireKind::ActionResponse if shape.0 && shape.1 && shape.2.is_some() => {
                match self.status {
                    Some(PocStatus::Accepted) if self.error_code.is_none() => Ok(()),
                    Some(PocStatus::Rejected) if self.error_code.is_some() => Ok(()),
                    _ => Err("wire action result shape is invalid"),
                }
            }
            _ => Err("wire message shape does not match its kind"),
        }
    }
}

pub(super) fn canonical_wire_json(message: &PocWireMessage) -> Result<String, &'static str> {
    message.validate()?;
    let encoded = serde_json::to_string(message).map_err(|_| "wire encoding failed")?;
    let decoded: PocWireMessage =
        serde_json::from_str(&encoded).map_err(|_| "wire round-trip decoding failed")?;
    if decoded != *message {
        return Err("wire round-trip changed the message");
    }
    decoded.validate()?;
    Ok(encoded)
}

fn validate_identity(value: &str) -> Result<(), &'static str> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-/".contains(&byte))
    {
        return Err("wire identity is empty, unsafe, or too long");
    }
    Ok(())
}
