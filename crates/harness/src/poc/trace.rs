// SPDX-License-Identifier: MIT

use super::contract::{PocAction, PocObservation, PocResponse, PocStatus};
use crate::protocol_artifact::{
    POC_ARTIFACT, POC_GENERATOR, POC_PROTOCOL_VERSION, POC_SCHEMA_DIGEST, POC_SCHEMA_SOURCE,
};

/// A trace record emitted for one POC boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceEvent {
    boundary: &'static str,
    tool: &'static str,
    correlation_id: String,
    instance_id: String,
    lease_id: &'static str,
    generation: u64,
    observation: PocObservation,
    action: Option<PocAction>,
    status: Option<PocStatus>,
    error_code: Option<&'static str>,
}

impl TraceEvent {
    pub(super) fn from_response(
        response: &PocResponse,
        boundary: &'static str,
        tool: &'static str,
        lease_id: &'static str,
    ) -> Self {
        Self {
            boundary,
            tool,
            correlation_id: response.correlation_id().to_owned(),
            instance_id: response.instance_id().to_owned(),
            lease_id,
            generation: response.generation(),
            observation: response.observation(),
            action: response.action(),
            status: response.status(),
            error_code: response.error_code(),
        }
    }

    /// Returns the owning boundary label.
    #[must_use]
    pub fn boundary(&self) -> &str {
        self.boundary
    }

    /// Returns the MCP tool label used for the operation.
    #[must_use]
    pub fn tool(&self) -> &str {
        self.tool
    }

    /// Returns the correlation identifier.
    #[must_use]
    pub fn correlation_id(&self) -> &str {
        &self.correlation_id
    }

    /// Returns the explicit instance identifier.
    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// Returns the fake gateway lease identity.
    #[must_use]
    pub const fn lease_id(&self) -> &str {
        self.lease_id
    }

    /// Returns the protocol version recorded at this boundary.
    #[must_use]
    pub const fn protocol_version(&self) -> &'static str {
        POC_PROTOCOL_VERSION
    }

    /// Returns the schema digest recorded at this boundary.
    #[must_use]
    pub const fn schema_digest(&self) -> &'static str {
        POC_SCHEMA_DIGEST
    }

    /// Returns the generation in this boundary's response.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns the bounded state observation.
    #[must_use]
    pub const fn observation(&self) -> PocObservation {
        self.observation
    }

    /// Returns the typed action, if this is an action response.
    #[must_use]
    pub const fn action(&self) -> Option<PocAction> {
        self.action
    }

    /// Returns the result status, if this is an action response.
    #[must_use]
    pub const fn status(&self) -> Option<PocStatus> {
        self.status
    }

    /// Returns the stable core error identity, if present.
    #[must_use]
    pub const fn error_code(&self) -> Option<&'static str> {
        self.error_code
    }

    pub(super) fn to_json(&self) -> String {
        let action = self.action.map_or_else(
            || String::from("null"),
            |action| {
                format!(
                    "{{\"action_id\":{},\"units\":{}}}",
                    json_string(action.action_id()),
                    action.units()
                )
            },
        );
        let status = self.status.map_or_else(
            || String::from("null"),
            |status| json_string(status.as_str()),
        );
        let error_code = self
            .error_code
            .map_or_else(|| String::from("null"), json_string);
        format!(
            "{{\"boundary\":{},\"tool\":{},\"protocol_version\":{},\"schema_digest\":{},\"provenance\":{{\"artifact\":{},\"source\":{},\"generator\":{}}},\"correlation_id\":{},\"instance_id\":{},\"lease_id\":{},\"generation\":{},\"observation\":{{\"available_units\":{},\"settled_effects\":{}}},\"action\":{},\"status\":{},\"error_code\":{}}}",
            json_string(self.boundary),
            json_string(self.tool),
            json_string(POC_PROTOCOL_VERSION),
            json_string(POC_SCHEMA_DIGEST),
            json_string(POC_ARTIFACT),
            json_string(POC_SCHEMA_SOURCE),
            json_string(POC_GENERATOR),
            json_string(&self.correlation_id),
            json_string(&self.instance_id),
            json_string(self.lease_id),
            self.generation,
            self.observation.available_units,
            self.observation.settled_effects,
            action,
            status,
            error_code,
        )
    }
}

fn json_string(value: &str) -> String {
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                output.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}
