// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use super::identity::{ExoIdentity, ExoIdentityError};
use super::{
    EXO_CAPABILITY_SCHEMA, EXO_CONTRACT_VERSION, EXO_MAX_EVENT_BYTES, EXO_MAX_RESPONSE_BYTES,
    EXO_MAX_TURN_TIME_MILLIS,
};
use crate::exo::protocol::{EXO_MAX_MAP_REQUEST_BYTES, EXO_MAX_STANDARD_REQUEST_BYTES};

/// Closed capability states prevent an operator from treating an unknown feature as available.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExoCapabilityState {
    Supported,
    Unsupported,
    Unverified,
}

/// The terminal choices accepted by the harness decision parser.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExoDecisionKind {
    Plan,
    Action,
    Wait,
    Reobserve,
    Recovery,
}

/// The upstream model runtime selected for a bound model.
///
/// The pinned upstream `runtimeFromModelBinding`
/// (`exoharness/typescript/model-runtime/responses.ts`, candidate
/// `b06869ab789dee3f80ca474b5fa89dbe47ccb859`) selects `AnthropicRuntime` for `claude*` bindings,
/// `ChatCompletionsRuntime` by default, and `ResponsesRuntime` only for the Responses-capable model
/// names mirrored by `preflight::responses_capable`. This contract admits only the
/// [`ExoRuntime::Responses`] runtime.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExoRuntime {
    Anthropic,
    ChatCompletions,
    Responses,
}

/// The three reviewed profile names. `map` and `expert` remain independently advertised.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExoProfile {
    Standard,
    Map,
    Expert,
}

/// Context continuity is separate from process/session identity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExoContextMode {
    Fresh,
    Continuity,
}

/// Initial platform support is deliberately narrow until a native run supplies evidence.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExoPlatform {
    LinuxX86_64,
}

/// Independent bridge limits. Each field is checked separately during preflight.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExoLimits {
    pub max_standard_request_bytes: u32,
    pub max_map_request_bytes: u32,
    pub max_response_bytes: u32,
    pub max_event_bytes: u32,
    pub max_turns: u32,
    pub max_turn_time_millis: u32,
    pub max_concurrency: u16,
    pub max_tool_round_trips: u16,
}

impl ExoLimits {
    /// The limits frozen by the current harness adapter.
    #[must_use]
    pub const fn reviewed() -> Self {
        Self {
            max_standard_request_bytes: EXO_MAX_STANDARD_REQUEST_BYTES as u32,
            max_map_request_bytes: EXO_MAX_MAP_REQUEST_BYTES as u32,
            max_response_bytes: EXO_MAX_RESPONSE_BYTES as u32,
            max_event_bytes: EXO_MAX_EVENT_BYTES as u32,
            max_turns: 1,
            max_turn_time_millis: EXO_MAX_TURN_TIME_MILLIS,
            max_concurrency: 1,
            max_tool_round_trips: 0,
        }
    }

    pub(super) fn validate(&self) -> Result<(), ExoDescriptorError> {
        if self.max_standard_request_bytes == 0
            || self.max_standard_request_bytes > EXO_MAX_STANDARD_REQUEST_BYTES as u32
        {
            return Err(ExoDescriptorError::InvalidLimit(
                "max_standard_request_bytes",
            ));
        }
        if self.max_map_request_bytes == 0
            || self.max_map_request_bytes > EXO_MAX_MAP_REQUEST_BYTES as u32
        {
            return Err(ExoDescriptorError::InvalidLimit("max_map_request_bytes"));
        }
        if self.max_response_bytes == 0 || self.max_response_bytes > EXO_MAX_RESPONSE_BYTES as u32 {
            return Err(ExoDescriptorError::InvalidLimit("max_response_bytes"));
        }
        if self.max_event_bytes == 0 || self.max_event_bytes > EXO_MAX_EVENT_BYTES as u32 {
            return Err(ExoDescriptorError::InvalidLimit("max_event_bytes"));
        }
        if self.max_turns == 0 || self.max_turns > 1 {
            return Err(ExoDescriptorError::InvalidLimit("max_turns"));
        }
        if self.max_turn_time_millis == 0 || self.max_turn_time_millis > EXO_MAX_TURN_TIME_MILLIS {
            return Err(ExoDescriptorError::InvalidLimit("max_turn_time_millis"));
        }
        if self.max_concurrency == 0 || self.max_concurrency > 1 {
            return Err(ExoDescriptorError::InvalidLimit("max_concurrency"));
        }
        if self.max_tool_round_trips > 0 {
            return Err(ExoDescriptorError::InvalidLimit("max_tool_round_trips"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExoProfileSupport {
    pub standard: ExoCapabilityState,
    pub map: ExoCapabilityState,
    pub expert: ExoCapabilityState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExoLifecycleCapabilities {
    pub cancellation: ExoCapabilityState,
    pub recovery: ExoCapabilityState,
    pub idempotency: ExoCapabilityState,
    pub graceful_eof: ExoCapabilityState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExoEvidenceCapabilities {
    pub terminal_decision: ExoCapabilityState,
    pub turn_identity: ExoCapabilityState,
    pub event_usage: ExoCapabilityState,
    pub replay: ExoCapabilityState,
}

/// Closed, versioned capability descriptor exchanged during offline preflight.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExoCapabilityDescriptor {
    pub schema_version: String,
    pub contract_version: String,
    pub identity: ExoIdentity,
    pub decision_kinds: Vec<ExoDecisionKind>,
    pub profile_support: ExoProfileSupport,
    pub context_modes: Vec<ExoContextMode>,
    pub platforms: Vec<ExoPlatform>,
    pub limits: ExoLimits,
    pub lifecycle: ExoLifecycleCapabilities,
    pub evidence: ExoEvidenceCapabilities,
}

impl ExoCapabilityDescriptor {
    /// Builds the source-derived descriptor shipped with this repository.
    pub fn source_review() -> Result<Self, ExoIdentityError> {
        Ok(Self {
            schema_version: EXO_CAPABILITY_SCHEMA.to_owned(),
            contract_version: EXO_CONTRACT_VERSION.to_owned(),
            identity: ExoIdentity::source_only(Some("unbound".to_owned()))?,
            decision_kinds: vec![
                ExoDecisionKind::Plan,
                ExoDecisionKind::Action,
                ExoDecisionKind::Wait,
                ExoDecisionKind::Reobserve,
                ExoDecisionKind::Recovery,
            ],
            profile_support: ExoProfileSupport {
                standard: ExoCapabilityState::Supported,
                map: ExoCapabilityState::Unverified,
                expert: ExoCapabilityState::Unverified,
            },
            context_modes: vec![ExoContextMode::Fresh],
            platforms: vec![ExoPlatform::LinuxX86_64],
            limits: ExoLimits::reviewed(),
            lifecycle: ExoLifecycleCapabilities {
                cancellation: ExoCapabilityState::Unverified,
                recovery: ExoCapabilityState::Unverified,
                idempotency: ExoCapabilityState::Supported,
                graceful_eof: ExoCapabilityState::Supported,
            },
            evidence: ExoEvidenceCapabilities {
                terminal_decision: ExoCapabilityState::Supported,
                turn_identity: ExoCapabilityState::Unverified,
                event_usage: ExoCapabilityState::Unverified,
                replay: ExoCapabilityState::Unverified,
            },
        })
    }

    pub fn validate(&self) -> Result<(), ExoDescriptorError> {
        if self.schema_version != EXO_CAPABILITY_SCHEMA {
            return Err(ExoDescriptorError::SchemaMismatch);
        }
        if self.contract_version != EXO_CONTRACT_VERSION {
            return Err(ExoDescriptorError::ContractMismatch);
        }
        self.identity
            .validate()
            .map_err(ExoDescriptorError::Identity)?;
        if self.decision_kinds.is_empty() || has_duplicates(&self.decision_kinds) {
            return Err(ExoDescriptorError::InvalidDecisionKinds);
        }
        if !self.decision_kinds.contains(&ExoDecisionKind::Action)
            || self.profile_support.standard != ExoCapabilityState::Supported
        {
            return Err(ExoDescriptorError::StandardUnavailable);
        }
        if !self.context_modes.contains(&ExoContextMode::Fresh)
            || has_duplicates(&self.context_modes)
        {
            return Err(ExoDescriptorError::FreshContextUnavailable);
        }
        if !self.platforms.contains(&ExoPlatform::LinuxX86_64) || has_duplicates(&self.platforms) {
            return Err(ExoDescriptorError::PlatformUnavailable);
        }
        self.limits.validate()
    }
}

fn has_duplicates<T: PartialEq>(values: &[T]) -> bool {
    values
        .iter()
        .enumerate()
        .any(|(index, value)| values[index + 1..].contains(value))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExoDescriptorError {
    SchemaMismatch,
    ContractMismatch,
    Identity(ExoIdentityError),
    InvalidDecisionKinds,
    StandardUnavailable,
    FreshContextUnavailable,
    PlatformUnavailable,
    InvalidLimit(&'static str),
}

impl std::fmt::Display for ExoDescriptorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::SchemaMismatch => "Exo capability schema is unsupported",
            Self::ContractMismatch => "Exo capability contract is unsupported",
            Self::Identity(_) => "Exo capability identity is invalid",
            Self::InvalidDecisionKinds => "Exo capability decision kinds are empty or duplicated",
            Self::StandardUnavailable => "Exo standard profile does not support action decisions",
            Self::FreshContextUnavailable => "Exo fresh context mode is missing or duplicated",
            Self::PlatformUnavailable => "Exo capability has no supported platform",
            Self::InvalidLimit(_) => "Exo capability limit is outside the reviewed bound",
        })
    }
}

impl std::error::Error for ExoDescriptorError {}
