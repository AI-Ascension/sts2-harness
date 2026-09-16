// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use super::descriptor::{
    ExoCapabilityDescriptor, ExoCapabilityState, ExoContextMode, ExoDescriptorError, ExoLimits,
    ExoPlatform, ExoProfile, ExoRuntime,
};
use super::identity::{ExoIdentity, ExoIdentityError};
use super::restricted::{ExoRestrictedError, ExoRestrictedProfile};
use super::{EXO_CONTRACT_VERSION, EXO_SOURCE_REVISION};

#[path = "preflight_checks.rs"]
mod checks;
use checks::{compare_limits, compare_optional_identity, require_minimum_capabilities};

/// Operator-trusted values required to admit one executable profile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExoTrustedConfiguration {
    pub identity: ExoIdentity,
    pub platform: ExoPlatform,
    pub profile: ExoProfile,
    pub context_mode: ExoContextMode,
    pub runtime: ExoRuntime,
    pub limits: ExoLimits,
    pub restricted: ExoRestrictedProfile,
}

/// Reports whether pinned upstream `modelRequiresResponsesApi`
/// (`exoharness/typescript/model-runtime/responses.ts`, candidate
/// `b06869ab789dee3f80ca474b5fa89dbe47ccb859`) would select the Responses runtime for a binding:
/// `o1-pro`/`o3-pro`/`gpt-5-pro`, `gpt-5.N` minor >= 3, or a `gpt-5*` binding containing `-codex`.
#[must_use]
pub fn responses_capable(model_binding: &str) -> bool {
    let lower = model_binding.to_ascii_lowercase();
    let gpt5_minor = lower
        .strip_prefix("gpt-5.")
        .map(|rest| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
        })
        .and_then(|digits| digits.parse::<u64>().ok());
    lower.starts_with("o1-pro")
        || lower.starts_with("o3-pro")
        || lower.starts_with("gpt-5-pro")
        || gpt5_minor.is_some_and(|minor| minor >= 3)
        || (lower.starts_with("gpt-5") && lower.contains("-codex"))
}

/// Reports whether the pinned upstream provider route can reach the Responses API.
///
/// The upstream binding carries a model and optional `baseUrl`; when that URL contains
/// `openrouter.ai`, `runtimeFromModelBinding` selects `ChatCompletionsRuntime` before it evaluates
/// the model predicate. The harness also requires the explicit provider and a reviewed OpenAI
/// endpoint so an unknown or provider-compatible route cannot be mistaken for Responses support.
#[must_use]
pub fn responses_routing_capable(provider: &str, endpoint: &str) -> bool {
    let provider = provider.to_ascii_lowercase();
    let endpoint = endpoint.to_ascii_lowercase();
    if provider != "openai" || endpoint.contains("openrouter.ai") {
        return false;
    }
    let Some(rest) = endpoint.strip_prefix("https://") else {
        return false;
    };
    rest.split('/')
        .next()
        .is_some_and(|host| host == "api.openai.com")
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
        self.restricted
            .validate()
            .map_err(ExoPreflightError::InvalidRestrictedProfile)?;
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
    pub runtime: ExoRuntime,
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
    if trusted.runtime != ExoRuntime::Responses {
        return Err(ExoPreflightError::RuntimeUnsupported);
    }
    let provider = trusted
        .identity
        .provider
        .as_deref()
        .ok_or(ExoPreflightError::MissingIdentity)?;
    let endpoint = trusted
        .identity
        .endpoint
        .as_deref()
        .ok_or(ExoPreflightError::MissingIdentity)?;
    if !responses_routing_capable(provider, endpoint) {
        return Err(ExoPreflightError::RoutingNotResponsesCapable);
    }
    // Check the route before the model predicate: upstream resolves OpenRouter (and other
    // non-Responses routes) to ChatCompletions regardless of whether the model name itself would
    // otherwise satisfy `modelRequiresResponsesApi`.
    let model_binding = trusted
        .identity
        .model_binding
        .as_deref()
        .ok_or(ExoPreflightError::MissingIdentity)?;
    if !responses_capable(model_binding) {
        return Err(ExoPreflightError::ModelBindingNotResponsesCapable);
    }
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
    // Bind the inspected deployment to the operator's pin before evaluating anything the deployment
    // claims about itself. A swapped artifact, a repinned digest or a wrong instance identity is
    // then refused as an identity mismatch instead of being masked by an unrelated capability gate.
    compare_optional_identity(descriptor, trusted)?;
    require_minimum_capabilities(descriptor)?;
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
        runtime: trusted.runtime,
        limits: trusted.limits.clone(),
        model_calls: 0,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExoPreflightError {
    InvalidDescriptor(ExoDescriptorError),
    InvalidIdentity(ExoIdentityError),
    InvalidLimits(ExoDescriptorError),
    InvalidRestrictedProfile(ExoRestrictedError),
    MissingIdentity,
    UnreviewedSourceRevision,
    ContractMismatch,
    IdentityMismatch(&'static str),
    UnboundIdentity(&'static str),
    RequiredCapability(&'static str),
    RuntimeUnsupported,
    ModelBindingNotResponsesCapable,
    RoutingNotResponsesCapable,
    PlatformUnsupported,
    ProfileUnsupported,
    ContextUnsupported,
    LimitExceeded(&'static str),
}

impl std::fmt::Display for ExoPreflightError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDescriptor(_) => {
                formatter.write_str("Exo capability descriptor failed validation")
            }
            Self::InvalidIdentity(_) => {
                formatter.write_str("trusted Exo identity failed validation")
            }
            Self::InvalidLimits(_) => formatter.write_str("trusted Exo limits failed validation"),
            Self::InvalidRestrictedProfile(_) => {
                formatter.write_str("trusted Exo restricted profile failed validation")
            }
            Self::MissingIdentity => formatter.write_str("trusted Exo identity is incomplete"),
            Self::UnreviewedSourceRevision => formatter
                .write_str("Exo source revision is not the reviewed candidate manifest pin"),
            Self::ContractMismatch => formatter.write_str("Exo contract versions do not match"),
            Self::IdentityMismatch(axis) => write!(
                formatter,
                "advertised Exo {axis} differs from the operator-trusted pin"
            ),
            Self::UnboundIdentity(axis) => write!(
                formatter,
                "the inspected Exo deployment did not bind the pinned {axis} identity"
            ),
            Self::RequiredCapability(_) => {
                formatter.write_str("Exo minimum admission capability is not supported")
            }
            Self::RuntimeUnsupported => {
                formatter.write_str("trusted Exo runtime is not the reviewed Responses runtime")
            }
            Self::ModelBindingNotResponsesCapable => {
                formatter.write_str("trusted Exo model binding cannot select the Responses runtime")
            }
            Self::RoutingNotResponsesCapable => formatter
                .write_str("trusted Exo provider endpoint cannot select the Responses runtime"),
            Self::PlatformUnsupported => {
                formatter.write_str("requested Exo platform is unsupported")
            }
            Self::ProfileUnsupported => {
                formatter.write_str("requested Exo profile is not supported")
            }
            Self::ContextUnsupported => {
                formatter.write_str("requested Exo context mode is unsupported")
            }
            Self::LimitExceeded(_) => {
                formatter.write_str("trusted Exo limit exceeds the advertised capability")
            }
        }
    }
}

impl std::error::Error for ExoPreflightError {}
