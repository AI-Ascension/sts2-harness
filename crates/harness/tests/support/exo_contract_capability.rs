// SPDX-License-Identifier: MIT

use super::*;
use serde_json::Value;

pub(super) fn execute_capability_vectors(vectors: &[Value]) -> Vec<String> {
    let mut consumed = Vec::new();
    let base = ExoCapabilityDescriptor::source_review().expect("source descriptor");
    let identity = complete_identity();
    let trusted = ExoTrustedConfiguration {
        identity,
        platform: ExoPlatform::LinuxX86_64,
        profile: ExoProfile::Standard,
        context_mode: ExoContextMode::Fresh,
        runtime: ExoRuntime::Responses,
        limits: ExoLimits::reviewed(),
        restricted: restricted_profile(),
    };
    for vector in vectors {
        let field = vector["field"].as_str().expect("capability vector field");
        consumed.push(
            vector["name"]
                .as_str()
                .expect("capability vector name")
                .to_owned(),
        );
        if field.starts_with("evidence.") || field.starts_with("lifecycle.") {
            assert_eq!(
                vector["downgrade"].as_str(),
                Some("unsupported"),
                "{field} must fail closed when unsupported"
            );
        }
        let mut descriptor = base.clone();
        enable_minimum_capabilities(&mut descriptor);
        let required = match field {
            "evidence.terminal_decision" => {
                descriptor.evidence.terminal_decision = ExoCapabilityState::Unsupported;
                "evidence.terminal_decision"
            }
            "evidence.turn_identity" => {
                descriptor.evidence.turn_identity = ExoCapabilityState::Unsupported;
                "evidence.turn_identity"
            }
            "lifecycle.graceful_eof" => {
                descriptor.lifecycle.graceful_eof = ExoCapabilityState::Unsupported;
                "lifecycle.graceful_eof"
            }
            "lifecycle.idempotency" => {
                descriptor.lifecycle.idempotency = ExoCapabilityState::Unsupported;
                "lifecycle.idempotency"
            }
            "lifecycle.cancellation" => {
                descriptor.lifecycle.cancellation = ExoCapabilityState::Unsupported;
                "lifecycle.cancellation"
            }
            "lifecycle.recovery" => {
                descriptor.lifecycle.recovery = ExoCapabilityState::Unsupported;
                "lifecycle.recovery"
            }
            "profile_support.map" => {
                assert_eq!(
                    vector["expect"].as_str(),
                    Some("profile_unsupported"),
                    "{field} must record an explicit unsupported-profile expectation"
                );
                let mut map = trusted.clone();
                map.profile = ExoProfile::Map;
                assert_eq!(
                    preflight_with_identity(descriptor, &map),
                    Err(ExoPreflightError::ProfileUnsupported),
                    "{field} must not admit an unverified profile"
                );
                continue;
            }
            "profile_support.expert" => {
                assert_eq!(
                    vector["expect"].as_str(),
                    Some("profile_unsupported"),
                    "{field} must record an explicit unsupported-profile expectation"
                );
                let mut expert = trusted.clone();
                expert.profile = ExoProfile::Expert;
                assert_eq!(
                    preflight_with_identity(descriptor, &expert),
                    Err(ExoPreflightError::ProfileUnsupported),
                    "{field} must not admit an unverified profile"
                );
                continue;
            }
            "context_modes.continuity" => {
                assert_eq!(
                    vector["expect"].as_str(),
                    Some("context_unsupported"),
                    "{field} must record an explicit unsupported-context expectation"
                );
                let mut continuity = trusted.clone();
                continuity.context_mode = ExoContextMode::Continuity;
                assert_eq!(
                    preflight_with_identity(descriptor, &continuity),
                    Err(ExoPreflightError::ContextUnsupported),
                    "{field} must not admit an absent context mode"
                );
                continue;
            }
            "schema_version" => {
                assert_eq!(
                    vector["expect"].as_str(),
                    Some("invalid_descriptor_schema"),
                    "{field} must record an explicit incompatible-schema expectation"
                );
                descriptor.schema_version = String::from("sts2.exo-capability-v0");
                assert_eq!(
                    preflight_with_identity(descriptor, &trusted),
                    Err(ExoPreflightError::InvalidDescriptor(
                        ExoDescriptorError::SchemaMismatch
                    )),
                    "{field} must reject an incompatible capability schema"
                );
                continue;
            }
            "contract_version" => {
                assert_eq!(
                    vector["expect"].as_str(),
                    Some("invalid_descriptor_contract"),
                    "{field} must record an explicit incompatible-contract expectation"
                );
                descriptor.contract_version = String::from("sts2-exo-bridge-v0");
                assert_eq!(
                    preflight_with_identity(descriptor, &trusted),
                    Err(ExoPreflightError::InvalidDescriptor(
                        ExoDescriptorError::ContractMismatch
                    )),
                    "{field} must reject an incompatible contract version"
                );
                continue;
            }
            "profile_support.standard" => {
                assert_eq!(
                    vector["expect"].as_str(),
                    Some("invalid_descriptor_standard"),
                    "{field} must record an explicit unsupported-standard expectation"
                );
                descriptor.profile_support.standard = ExoCapabilityState::Unsupported;
                assert_eq!(
                    preflight_with_identity(descriptor, &trusted),
                    Err(ExoPreflightError::InvalidDescriptor(
                        ExoDescriptorError::StandardUnavailable
                    )),
                    "{field} must reject an unavailable standard profile before preflight"
                );
                continue;
            }
            other => unreachable!("unhandled capability conformance vector {other}"),
        };
        assert_eq!(
            preflight_with_identity(descriptor, &trusted),
            Err(ExoPreflightError::RequiredCapability(required))
        );
    }
    consumed
}
