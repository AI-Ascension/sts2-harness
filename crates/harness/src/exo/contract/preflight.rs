// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use super::descriptor::{
    ExoCapabilityDescriptor, ExoCapabilityState, ExoContextMode, ExoDescriptorError, ExoLimits,
    ExoPlatform, ExoProfile,
};
use super::identity::{ExoIdentity, ExoIdentityError};
use super::{EXO_CONTRACT_VERSION, EXO_SOURCE_REVISION};

/// Operator-trusted values required to admit one executable profile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExoTrustedConfiguration {
    pub identity: ExoIdentity,
    pub platform: ExoPlatform,
    pub profile: ExoProfile,
    pub context_mode: ExoContextMode,
    pub limits: ExoLimits,
}

impl ExoTrustedConfiguration {
    pub fn validate(&self) -> Result<(), ExoPreflightError> {
        self.identity
            .validate()
            .map_err(ExoPreflightError::InvalidIdentity)?;
        if self.identity.source_revision != EXO_SOURCE_REVISION {
            return Err(ExoPreflightError::UnreviewedSourceRevision);
        }
        if !self.identity.is_complete() {
            return Err(ExoPreflightError::MissingIdentity);
        }
        self.limits
            .validate()
            .map_err(ExoPreflightError::InvalidLimits)
    }
}

/// The result of an offline preflight. `model_calls` is always zero by construction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExoPreflightReport {
    pub contract_version: String,
    pub identity: ExoIdentity,
    pub platform: ExoPlatform,
    pub profile: ExoProfile,
    pub context_mode: ExoContextMode,
    pub limits: ExoLimits,
    pub model_calls: u32,
}

/// Validates a descriptor against trusted operator metadata without contacting Exo or a model.
pub fn preflight(
    descriptor: &ExoCapabilityDescriptor,
    trusted: &ExoTrustedConfiguration,
) -> Result<ExoPreflightReport, ExoPreflightError> {
    descriptor
        .validate()
        .map_err(ExoPreflightError::InvalidDescriptor)?;
    trusted.validate()?;
    if descriptor.identity.source_revision != EXO_SOURCE_REVISION
        || trusted.identity.source_revision != EXO_SOURCE_REVISION
    {
        return Err(ExoPreflightError::UnreviewedSourceRevision);
    }
    if descriptor.identity.source_revision != trusted.identity.source_revision {
        return Err(ExoPreflightError::IdentityMismatch("source_revision"));
    }
    if descriptor.identity.contract_version != trusted.identity.contract_version {
        return Err(ExoPreflightError::ContractMismatch);
    }
    require_minimum_capabilities(descriptor)?;
    compare_optional_identity(descriptor, trusted)?;
    if !descriptor.platforms.contains(&trusted.platform) {
        return Err(ExoPreflightError::PlatformUnsupported);
    }
    let profile_state = match trusted.profile {
        ExoProfile::Standard => descriptor.profile_support.standard,
        ExoProfile::Map => descriptor.profile_support.map,
        ExoProfile::Expert => descriptor.profile_support.expert,
    };
    if profile_state != ExoCapabilityState::Supported {
        return Err(ExoPreflightError::ProfileUnsupported);
    }
    if trusted.context_mode == ExoContextMode::Continuity
        && !descriptor
            .context_modes
            .contains(&ExoContextMode::Continuity)
    {
        return Err(ExoPreflightError::ContextUnsupported);
    }
    compare_limits(&descriptor.limits, &trusted.limits)?;
    Ok(ExoPreflightReport {
        contract_version: EXO_CONTRACT_VERSION.to_owned(),
        identity: trusted.identity.clone(),
        platform: trusted.platform,
        profile: trusted.profile,
        context_mode: trusted.context_mode,
        limits: trusted.limits.clone(),
        model_calls: 0,
    })
}

fn require_minimum_capabilities(
    descriptor: &ExoCapabilityDescriptor,
) -> Result<(), ExoPreflightError> {
    let required = [
        (
            "evidence.terminal_decision",
            descriptor.evidence.terminal_decision,
        ),
        ("lifecycle.graceful_eof", descriptor.lifecycle.graceful_eof),
        ("lifecycle.idempotency", descriptor.lifecycle.idempotency),
    ];
    required
        .into_iter()
        .find(|(_, state)| *state != ExoCapabilityState::Supported)
        .map_or(Ok(()), |(name, _)| {
            Err(ExoPreflightError::RequiredCapability(name))
        })
}

fn compare_limits(advertised: &ExoLimits, trusted: &ExoLimits) -> Result<(), ExoPreflightError> {
    let limits = [
        (
            "max_standard_request_bytes",
            trusted.max_standard_request_bytes,
            advertised.max_standard_request_bytes,
        ),
        (
            "max_map_request_bytes",
            trusted.max_map_request_bytes,
            advertised.max_map_request_bytes,
        ),
        (
            "max_response_bytes",
            trusted.max_response_bytes,
            advertised.max_response_bytes,
        ),
        (
            "max_event_bytes",
            trusted.max_event_bytes,
            advertised.max_event_bytes,
        ),
        ("max_turns", trusted.max_turns, advertised.max_turns),
        (
            "max_turn_time_millis",
            trusted.max_turn_time_millis,
            advertised.max_turn_time_millis,
        ),
        (
            "max_concurrency",
            u32::from(trusted.max_concurrency),
            u32::from(advertised.max_concurrency),
        ),
        (
            "max_tool_round_trips",
            u32::from(trusted.max_tool_round_trips),
            u32::from(advertised.max_tool_round_trips),
        ),
    ];
    limits
        .into_iter()
        .find(|(_, requested, maximum)| requested > maximum)
        .map_or(Ok(()), |(name, _, _)| {
            Err(ExoPreflightError::LimitExceeded(name))
        })
}

fn compare_optional_identity(
    descriptor: &ExoCapabilityDescriptor,
    trusted: &ExoTrustedConfiguration,
) -> Result<(), ExoPreflightError> {
    let pairs = [
        (
            "package_digest",
            descriptor.identity.package_digest.as_ref(),
            trusted.identity.package_digest.as_ref(),
        ),
        (
            "extension_digest",
            descriptor.identity.extension_digest.as_ref(),
            trusted.identity.extension_digest.as_ref(),
        ),
        (
            "bridge_digest",
            descriptor.identity.bridge_digest.as_ref(),
            trusted.identity.bridge_digest.as_ref(),
        ),
        (
            "model_binding",
            descriptor.identity.model_binding.as_ref(),
            trusted.identity.model_binding.as_ref(),
        ),
        (
            "prompt_digest",
            descriptor.identity.prompt_digest.as_ref(),
            trusted.identity.prompt_digest.as_ref(),
        ),
        (
            "tool_digest",
            descriptor.identity.tool_digest.as_ref(),
            trusted.identity.tool_digest.as_ref(),
        ),
        (
            "config_digest",
            descriptor.identity.config_digest.as_ref(),
            trusted.identity.config_digest.as_ref(),
        ),
        (
            "native_instance_id",
            descriptor.identity.native_instance_id.as_ref(),
            trusted.identity.native_instance_id.as_ref(),
        ),
    ];
    for (name, advertised, expected) in pairs {
        if advertised != expected {
            return Err(ExoPreflightError::IdentityMismatch(name));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExoPreflightError {
    InvalidDescriptor(ExoDescriptorError),
    InvalidIdentity(ExoIdentityError),
    InvalidLimits(ExoDescriptorError),
    MissingIdentity,
    UnreviewedSourceRevision,
    ContractMismatch,
    IdentityMismatch(&'static str),
    RequiredCapability(&'static str),
    PlatformUnsupported,
    ProfileUnsupported,
    ContextUnsupported,
    LimitExceeded(&'static str),
}

impl std::fmt::Display for ExoPreflightError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDescriptor(_) => "Exo capability descriptor failed validation",
            Self::InvalidIdentity(_) => "trusted Exo identity failed validation",
            Self::InvalidLimits(_) => "trusted Exo limits failed validation",
            Self::MissingIdentity => "trusted Exo identity is incomplete",
            Self::UnreviewedSourceRevision => {
                "Exo source revision is not the reviewed candidate manifest pin"
            }
            Self::ContractMismatch => "Exo contract versions do not match",
            Self::IdentityMismatch(_) => "advertised and trusted Exo identities differ",
            Self::RequiredCapability(_) => "Exo minimum admission capability is not supported",
            Self::PlatformUnsupported => "requested Exo platform is unsupported",
            Self::ProfileUnsupported => "requested Exo profile is not supported",
            Self::ContextUnsupported => "requested Exo context mode is unsupported",
            Self::LimitExceeded(_) => "trusted Exo limit exceeds the advertised capability",
        })
    }
}

impl std::error::Error for ExoPreflightError {}
