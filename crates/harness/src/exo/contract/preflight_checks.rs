// SPDX-License-Identifier: MIT

use super::super::descriptor::{ExoCapabilityDescriptor, ExoCapabilityState, ExoLimits};
use super::{ExoPreflightError, ExoTrustedConfiguration};

pub(super) fn require_minimum_capabilities(
    descriptor: &ExoCapabilityDescriptor,
) -> Result<(), ExoPreflightError> {
    let required = [
        (
            "evidence.terminal_decision",
            descriptor.evidence.terminal_decision,
        ),
        ("evidence.turn_identity", descriptor.evidence.turn_identity),
        ("lifecycle.graceful_eof", descriptor.lifecycle.graceful_eof),
        ("lifecycle.idempotency", descriptor.lifecycle.idempotency),
        ("lifecycle.cancellation", descriptor.lifecycle.cancellation),
        ("lifecycle.recovery", descriptor.lifecycle.recovery),
    ];
    required
        .into_iter()
        .find(|(_, state)| *state != ExoCapabilityState::Supported)
        .map_or(Ok(()), |(name, _)| {
            Err(ExoPreflightError::RequiredCapability(name))
        })
}

pub(super) fn compare_limits(
    advertised: &ExoLimits,
    trusted: &ExoLimits,
) -> Result<(), ExoPreflightError> {
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

/// Cross-checks the identity the deployment advertises against the operator's pin.
///
/// The advertised identity is what was actually inspected; the pin is what the operator requires.
/// An axis the pin requires but the inspection did not bind is refused as `UnboundIdentity`, so a
/// deployment can never be admitted on a pinned declaration that no artifact backed.
pub(super) fn compare_optional_identity(
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
            "provider",
            descriptor.identity.provider.as_ref(),
            trusted.identity.provider.as_ref(),
        ),
        (
            "endpoint",
            descriptor.identity.endpoint.as_ref(),
            trusted.identity.endpoint.as_ref(),
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
        let Some(expected) = expected else {
            continue;
        };
        match advertised {
            Some(advertised) if advertised == expected => {}
            Some(_) => return Err(ExoPreflightError::IdentityMismatch(name)),
            None => return Err(ExoPreflightError::UnboundIdentity(name)),
        }
    }
    Ok(())
}
