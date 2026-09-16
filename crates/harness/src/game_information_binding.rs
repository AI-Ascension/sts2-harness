// SPDX-License-Identifier: MIT
use crate::game_information_validation::decode_strict;
use crate::sha256_hex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::OnceLock;

pub const LOOKUP_BINDING_PROFILE: &str = "game-information-lookup-binding-v1";
pub const LOOKUP_BINDING_SCHEMA_DIGEST: &str =
    "f10f9af01d6be1de104069ba842e7971971e88f27553e782e81174ee7aa1cd58";
const MAX_REOBSERVE_CALLS: usize = 2;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookupScope {
    pub project_id: String,
    pub run_id: String,
    pub episode_id: String,
    pub agent_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LookupBindingOperation {
    Discovery,
    Observe,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookupBindingRequest {
    pub operation: LookupBindingOperation,
    pub scope: LookupScope,
    pub authority_epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LookupBindingContext {
    pub instance_id: String,
    pub scope: LookupScope,
    pub authority_epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LookupBinding {
    pub binding_id: String,
    pub game_profile: String,
    pub content_manifest_id: String,
    pub locale: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LookupObservation {
    pub observation_id: String,
    pub snapshot_id: String,
    pub state_generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LookupBindingError {
    Invalid,
    UnsupportedVersion,
    InvalidIdentity,
    DeniedScope,
    MissingCapability,
    MixedBinding,
    StaleSnapshot,
    ReobserveUnavailable,
    DiscoveryRequired,
    NativeUnavailable,
    Transport,
}

impl std::fmt::Display for LookupBindingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "game-information lookup binding: {self:?}")
    }
}

impl std::error::Error for LookupBindingError {}

/// Boundary adapter implemented by the runtime. It only carries the closed
/// request identity; it cannot supply locale, profile, manifest, or snapshots.
pub trait LookupBindingPort {
    fn lookup_binding(
        &mut self,
        request: &LookupBindingRequest,
    ) -> Result<Value, LookupBindingError>;
}

/// One discovered binding and its latest observation. Re-observation never
/// changes the discovery identity.
pub struct LookupBindingSession {
    context: LookupBindingContext,
    discovered: Option<LookupBinding>,
    observation: Option<LookupObservation>,
}

impl LookupBindingSession {
    #[must_use]
    pub fn new(context: LookupBindingContext) -> Self {
        Self {
            context,
            discovered: None,
            observation: None,
        }
    }

    pub fn binding(&self) -> Option<&LookupBinding> {
        self.discovered.as_ref()
    }

    pub fn observation(&self) -> Option<&LookupObservation> {
        self.observation.as_ref()
    }

    pub fn discover<P: LookupBindingPort>(
        &mut self,
        port: &mut P,
    ) -> Result<&LookupBinding, LookupBindingError> {
        if self.discovered.is_some() {
            return Err(LookupBindingError::Invalid);
        }
        let response = port.lookup_binding(&self.request(LookupBindingOperation::Discovery))?;
        let decoded = self.decode(&response)?;
        if decoded.kind != "lookup_binding_discovery_response"
            || decoded.state != "not_yet_observed"
            || decoded.observation.is_some()
        {
            return Err(LookupBindingError::Invalid);
        }
        self.discovered = Some(decoded.binding);
        self.discovered.as_ref().ok_or(LookupBindingError::Invalid)
    }

    /// Reads one fresh observation. A stale response is discarded and exactly
    /// one same-binding re-observation is attempted; exhaustion is terminal.
    pub fn observe<P: LookupBindingPort>(
        &mut self,
        port: &mut P,
    ) -> Result<&LookupObservation, LookupBindingError> {
        let discovered = self
            .discovered
            .as_ref()
            .cloned()
            .ok_or(LookupBindingError::DiscoveryRequired)?;
        let retained_generation = self
            .observation
            .as_ref()
            .map_or(0, |observation| observation.state_generation);
        for attempt in 0..MAX_REOBSERVE_CALLS {
            let response = port.lookup_binding(&self.request(LookupBindingOperation::Observe))?;
            let decoded = self.decode(&response)?;
            if decoded.binding != discovered {
                return Err(LookupBindingError::InvalidIdentity);
            }
            match (decoded.state.as_str(), decoded.observation) {
                ("observed", Some(observation))
                    if observation.state_generation >= retained_generation =>
                {
                    if self.observation.as_ref().is_some_and(|previous| {
                        previous.observation_id == observation.observation_id
                    }) {
                        return Err(LookupBindingError::StaleSnapshot);
                    }
                    self.observation = Some(observation);
                    return self.observation.as_ref().ok_or(LookupBindingError::Invalid);
                }
                ("observed", Some(_)) if attempt + 1 < MAX_REOBSERVE_CALLS => {
                    self.observation = None;
                }
                ("reobserve_required", None) if attempt + 1 < MAX_REOBSERVE_CALLS => {
                    self.observation = None;
                }
                ("reobserve_exhausted", None) => {
                    self.observation = None;
                    return Err(LookupBindingError::ReobserveUnavailable);
                }
                _ => return Err(LookupBindingError::StaleSnapshot),
            }
        }
        Err(LookupBindingError::ReobserveUnavailable)
    }

    fn request(&self, operation: LookupBindingOperation) -> LookupBindingRequest {
        LookupBindingRequest {
            operation,
            scope: self.context.scope.clone(),
            authority_epoch: self.context.authority_epoch,
        }
    }

    fn decode(&self, value: &Value) -> Result<Decoded, LookupBindingError> {
        schema(value)?;
        if value["protocol_version"] != LOOKUP_BINDING_PROFILE
            || value["schema_digest"] != LOOKUP_BINDING_SCHEMA_DIGEST
        {
            return Err(LookupBindingError::UnsupportedVersion);
        }
        let binding = &value["binding"];
        if binding.is_null() {
            return Err(error_code(value));
        }
        if binding["scope"] != json!(self.context.scope)
            || binding["instance_id"] != self.context.instance_id
            || binding["authority_epoch"] != self.context.authority_epoch
        {
            return Err(LookupBindingError::DeniedScope);
        }
        if value["discovery"]["required_capabilities"]["profile"] != LOOKUP_BINDING_PROFILE
            || value["discovery"]["required_capabilities"]["schema_digest"]
                != LOOKUP_BINDING_SCHEMA_DIGEST
        {
            return Err(LookupBindingError::MissingCapability);
        }
        let supplied = binding["binding_id"]
            .as_str()
            .ok_or(LookupBindingError::Invalid)?;
        let expected = binding_id(binding)?;
        if supplied != expected {
            return Err(LookupBindingError::InvalidIdentity);
        }
        let decoded = LookupBinding {
            binding_id: supplied.to_owned(),
            game_profile: string(binding, "game_profile")?,
            content_manifest_id: string(binding, "content_manifest_id")?,
            locale: string(binding, "locale")?,
        };
        let observation = if value["observation"].is_null() {
            None
        } else {
            let observation = &value["observation"];
            if observation["binding_id"] != supplied {
                return Err(LookupBindingError::MixedBinding);
            }
            Some(LookupObservation {
                observation_id: string(observation, "observation_id")?,
                snapshot_id: string(observation, "snapshot_id")?,
                state_generation: observation["state_generation"]
                    .as_u64()
                    .ok_or(LookupBindingError::Invalid)?,
            })
        };
        Ok(Decoded {
            kind: string(value, "kind")?,
            state: string(&value["discovery"], "observation_state")?,
            binding: decoded,
            observation,
        })
    }
}

struct Decoded {
    kind: String,
    state: String,
    binding: LookupBinding,
    observation: Option<LookupObservation>,
}

fn string(value: &Value, name: &str) -> Result<String, LookupBindingError> {
    value[name]
        .as_str()
        .map(str::to_owned)
        .ok_or(LookupBindingError::Invalid)
}

fn binding_id(binding: &Value) -> Result<String, LookupBindingError> {
    let scope = &binding["scope"];
    let input = json!({
        "agent_id": string(scope, "agent_id")?,
        "authority_epoch": binding["authority_epoch"].as_u64().ok_or(LookupBindingError::Invalid)?,
        "content_manifest_id": string(binding, "content_manifest_id")?,
        "episode_id": string(scope, "episode_id")?,
        "game_profile": string(binding, "game_profile")?,
        "locale": string(binding, "locale")?,
        "project_id": string(scope, "project_id")?,
        "run_id": string(scope, "run_id")?,
    });
    serde_json::to_vec(&input)
        .map(sha256_hex)
        .map_err(|_| LookupBindingError::Invalid)
}

fn schema(value: &Value) -> Result<(), LookupBindingError> {
    static VALIDATOR: OnceLock<Result<jsonschema::Validator, ()>> = OnceLock::new();
    let validator = VALIDATOR
        .get_or_init(|| {
            let source = include_str!(
                "../../../protocol-artifact/game-information-lookup-binding-v1/schema.json"
            );
            if sha256_hex(source) != LOOKUP_BINDING_SCHEMA_DIGEST {
                return Err(());
            }
            let schema = serde_json::from_str(source).map_err(|_| ())?;
            jsonschema::validator_for(&schema).map_err(|_| ())
        })
        .as_ref()
        .map_err(|_| LookupBindingError::Invalid)?;
    if !validator.is_valid(value) {
        return Err(LookupBindingError::Invalid);
    }
    Ok(())
}

fn error_code(value: &Value) -> LookupBindingError {
    match value["error"]["code"].as_str() {
        Some("unsupported_version") => LookupBindingError::UnsupportedVersion,
        Some("invalid_identity") => LookupBindingError::InvalidIdentity,
        Some("denied_scope") => LookupBindingError::DeniedScope,
        Some("missing_capability") => LookupBindingError::MissingCapability,
        Some("stale_snapshot") => LookupBindingError::StaleSnapshot,
        Some("mixed_binding") => LookupBindingError::MixedBinding,
        Some("reobserve_unavailable") => LookupBindingError::ReobserveUnavailable,
        _ => LookupBindingError::Invalid,
    }
}

/// Decode raw boundary bytes with the existing duplicate-member rejection.
pub fn decode_lookup_binding_response(bytes: &[u8]) -> Result<Value, LookupBindingError> {
    decode_strict(bytes).map_err(|_| LookupBindingError::Invalid)
}

#[cfg(test)]
#[path = "game_information_binding_tests.rs"]
mod tests;
