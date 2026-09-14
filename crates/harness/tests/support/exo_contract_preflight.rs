// SPDX-License-Identifier: MIT

use super::*;
use serde_json::json;

#[test]
fn capability_descriptor_is_closed_and_identity_axes_are_distinct() {
    let descriptor = ExoCapabilityDescriptor::source_review().expect("source descriptor is valid");
    let mut value = serde_json::to_value(descriptor).expect("descriptor serializes");
    value["unexpected"] = json!(true);
    assert!(serde_json::from_value::<ExoCapabilityDescriptor>(value).is_err());
    let mut identity = complete_identity();
    identity.source_revision = String::from("a").repeat(40);
    assert_ne!(
        identity.source_revision,
        identity.bridge_digest.expect("digest exists")
    );
}

#[test]
fn responses_runtime_precondition_is_enforced() {
    let mut descriptor =
        ExoCapabilityDescriptor::source_review().expect("source descriptor is valid");
    enable_minimum_capabilities(&mut descriptor);
    let identity = complete_identity();
    descriptor.identity = identity.clone();
    let trusted = ExoTrustedConfiguration {
        identity,
        platform: ExoPlatform::LinuxX86_64,
        profile: ExoProfile::Standard,
        context_mode: ExoContextMode::Fresh,
        runtime: ExoRuntime::Responses,
        limits: ExoLimits::reviewed(),
    };
    assert!(preflight(&descriptor, &trusted).is_ok());

    let mut wrong_runtime = trusted.clone();
    wrong_runtime.runtime = ExoRuntime::ChatCompletions;
    assert_eq!(
        preflight(&descriptor, &wrong_runtime),
        Err(ExoPreflightError::RuntimeUnsupported)
    );
    let mut anthropic_runtime = trusted.clone();
    anthropic_runtime.runtime = ExoRuntime::Anthropic;
    assert_eq!(
        preflight(&descriptor, &anthropic_runtime),
        Err(ExoPreflightError::RuntimeUnsupported)
    );

    let mut wrong_model = trusted;
    wrong_model.identity.model_binding = Some(String::from("gpt-4o"));
    assert_eq!(
        preflight(&descriptor, &wrong_model),
        Err(ExoPreflightError::ModelBindingNotResponsesCapable)
    );
}

#[test]
fn responses_preflight_rejects_openrouter_override_before_model_predicate() {
    let mut descriptor =
        ExoCapabilityDescriptor::source_review().expect("source descriptor is valid");
    enable_minimum_capabilities(&mut descriptor);
    let mut identity = complete_identity();
    identity.model_binding = Some(String::from("o3-pro"));
    descriptor.identity = identity.clone();
    let trusted = ExoTrustedConfiguration {
        identity,
        platform: ExoPlatform::LinuxX86_64,
        profile: ExoProfile::Standard,
        context_mode: ExoContextMode::Fresh,
        runtime: ExoRuntime::Responses,
        limits: ExoLimits::reviewed(),
    };

    let mut openrouter = trusted.clone();
    openrouter.identity.endpoint = Some(String::from("https://openrouter.ai/api/v1"));
    descriptor.identity = openrouter.identity.clone();
    assert_eq!(
        preflight(&descriptor, &openrouter),
        Err(ExoPreflightError::RoutingNotResponsesCapable),
        "OpenRouter must remain Chat Completions even for o3-pro"
    );

    let mut provider_override = trusted.clone();
    provider_override.identity.provider = Some(String::from("openrouter"));
    descriptor.identity = provider_override.identity.clone();
    assert_eq!(
        preflight(&descriptor, &provider_override),
        Err(ExoPreflightError::RoutingNotResponsesCapable)
    );

    let mut unknown_endpoint = trusted;
    unknown_endpoint.identity.endpoint = Some(String::from("https://models.example/v1"));
    descriptor.identity = unknown_endpoint.identity.clone();
    assert_eq!(
        preflight(&descriptor, &unknown_endpoint),
        Err(ExoPreflightError::RoutingNotResponsesCapable)
    );
}

#[test]
fn responses_routing_helper_requires_reviewed_openai_endpoint() {
    assert!(responses_routing_capable(
        "openai",
        "https://api.openai.com/v1"
    ));
    assert!(!responses_routing_capable(
        "openai",
        "https://openrouter.ai/api/v1"
    ));
    assert!(!responses_routing_capable(
        "openrouter",
        "https://api.openai.com/v1"
    ));
    assert!(!responses_routing_capable(
        "openai",
        "https://models.example/v1"
    ));
}

#[test]
fn responses_capable_mirrors_pinned_upstream_routing() {
    for model in [
        "o1-pro",
        "o1-pro-2025-01-01",
        "o3-pro",
        "gpt-5-pro",
        "gpt-5.3",
        "gpt-5.10",
        "gpt-5.3-codex",
        "gpt-5.2-codex",
        "GPT-5-PRO",
    ] {
        assert!(
            responses_capable(model),
            "{model} must select the Responses runtime"
        );
    }
    for model in [
        "",
        "gpt-4o",
        "gpt-5",
        "gpt-5.0",
        "gpt-5.2",
        "gpt-5-mini",
        "claude-3-5-sonnet",
        "o1",
        "o3-mini",
    ] {
        assert!(
            !responses_capable(model),
            "{model} must select Chat Completions or Anthropic"
        );
    }
}
