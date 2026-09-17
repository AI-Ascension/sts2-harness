// SPDX-License-Identifier: MIT
use serde::{Deserialize, Serialize};
use serde_json::json;

#[path = "game_information_binding_validation.rs"]
mod validation;
pub use validation::decode_lookup_binding_response;
#[path = "game_information_bootstrap.rs"]
pub mod game_information_bootstrap;
use validation::{Decoded, binding_id, error_code, schema, string};

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
    pub correlation_id: String,
}

/// Builds the one closed discovery request shared by runtime startup and the
/// retained binding session. Its identity is supplied only by the selected
/// authority owner.
pub fn discovery_request(scope: LookupScope, authority_epoch: u64) -> LookupBindingRequest {
    LookupBindingRequest {
        operation: LookupBindingOperation::Discovery,
        scope,
        authority_epoch,
        correlation_id: String::from("game-information-binding-discovery"),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LookupBindingContext {
    pub instance_id: String,
    pub scope: LookupScope,
    pub authority_epoch: u64,
    pub supported_capabilities: Vec<String>,
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

/// Boundary adapter implemented by the runtime. It only carries the closed
/// request identity; it cannot supply locale, profile, manifest, or snapshots.
pub trait LookupBindingPort {
    fn lookup_binding(
        &mut self,
        request: &LookupBindingRequest,
    ) -> Result<Vec<u8>, LookupBindingError>;
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
        let request = self.request(LookupBindingOperation::Discovery);
        let response = port.lookup_binding(&request)?;
        let decoded = self.decode(&response, &request.correlation_id, None)?;
        if decoded.kind != "lookup_binding_discovery_response"
            || decoded.state != "not_yet_observed"
            || decoded.observation.is_some()
            || decoded.error.is_some()
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
        let retained = self.observation.clone();
        for attempt in 0..MAX_REOBSERVE_CALLS {
            let request = self.request(LookupBindingOperation::Observe);
            let response = port.lookup_binding(&request)?;
            let decoded = match self.decode(&response, &request.correlation_id, retained.as_ref()) {
                Err(LookupBindingError::StaleSnapshot) if attempt + 1 < MAX_REOBSERVE_CALLS => {
                    self.observation = None;
                    continue;
                }
                result => result?,
            };
            if decoded.error == Some(LookupBindingError::ReobserveUnavailable) {
                self.observation = None;
                return Err(LookupBindingError::ReobserveUnavailable);
            }
            if decoded.kind != "lookup_binding_observation_response" {
                return Err(LookupBindingError::Invalid);
            }
            if decoded.binding != discovered {
                return Err(LookupBindingError::InvalidIdentity);
            }
            match (decoded.state.as_str(), decoded.observation) {
                ("observed", Some(observation)) => {
                    if retained.as_ref().is_some_and(|previous| {
                        observation.state_generation < previous.state_generation
                            || previous.observation_id == observation.observation_id
                    }) {
                        self.observation = None;
                        return Err(LookupBindingError::StaleSnapshot);
                    }
                    self.observation = Some(observation);
                    return self.observation.as_ref().ok_or(LookupBindingError::Invalid);
                }
                ("reobserve_required", None) if attempt + 1 < MAX_REOBSERVE_CALLS => {
                    self.observation = None;
                }
                ("reobserve_exhausted", None) => {
                    self.observation = None;
                    return Err(LookupBindingError::ReobserveUnavailable);
                }
                ("reobserve_required", None) => {
                    self.observation = None;
                    return Err(LookupBindingError::ReobserveUnavailable);
                }
                _ => return Err(LookupBindingError::StaleSnapshot),
            }
        }
        Err(LookupBindingError::ReobserveUnavailable)
    }

    fn request(&self, operation: LookupBindingOperation) -> LookupBindingRequest {
        match operation {
            LookupBindingOperation::Discovery => {
                discovery_request(self.context.scope.clone(), self.context.authority_epoch)
            }
            LookupBindingOperation::Observe => LookupBindingRequest {
                operation,
                scope: self.context.scope.clone(),
                authority_epoch: self.context.authority_epoch,
                correlation_id: String::from("game-information-binding-observe"),
            },
        }
    }

    fn decode(
        &self,
        bytes: &[u8],
        expected_correlation: &str,
        retained: Option<&LookupObservation>,
    ) -> Result<Decoded, LookupBindingError> {
        decode_lookup_binding_session_response(&self.context, bytes, expected_correlation, retained)
    }
}

include!("game_information_binding_decode.rs");

#[cfg(test)]
#[path = "game_information_binding_tests.rs"]
mod tests;
