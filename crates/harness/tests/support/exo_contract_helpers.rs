// SPDX-License-Identifier: MIT

use sts2_harness::{
    EXO_SOURCE_REVISION, ExoCapabilityDescriptor, ExoCapabilityState, ExoControlIdentity,
    ExoIdentity, ExoPreflightError, ExoRestrictedProfile, ExoTrustedConfiguration, preflight,
};

pub fn restricted_profile() -> ExoRestrictedProfile {
    ExoRestrictedProfile::reviewed_private("/var/lib/sts2-harness/exo-contract-140")
}

pub fn complete_identity() -> ExoIdentity {
    ExoIdentity {
        source_revision: EXO_SOURCE_REVISION.to_owned(),
        package_digest: Some(String::from("a").repeat(64)),
        extension_digest: Some(String::from("b").repeat(64)),
        bridge_digest: Some(String::from("c").repeat(64)),
        model_binding: Some(String::from("gpt-5-pro")),
        provider: Some(String::from("openai")),
        endpoint: Some(String::from("https://api.openai.com/v1")),
        prompt_digest: Some(String::from("d").repeat(64)),
        tool_digest: Some(String::from("e").repeat(64)),
        config_digest: Some(String::from("f").repeat(64)),
        contract_version: String::from("sts2-exo-bridge-v1"),
        native_instance_id: Some(String::from("native-1")),
    }
}

pub fn control_identity(run_id: &str) -> ExoControlIdentity {
    ExoControlIdentity {
        run_id: run_id.to_owned(),
        episode_id: String::from("episode-1"),
        model_execution_id: String::from("execution-1"),
        agent_id: String::from("agent-1"),
        conversation_id: String::from("conversation-1"),
        session_id: String::from("session-1"),
        turn_id: String::from("turn-1"),
        idempotency_key: String::from("idem-1"),
    }
}

pub fn preflight_with_identity(
    descriptor: ExoCapabilityDescriptor,
    trusted: &ExoTrustedConfiguration,
) -> Result<sts2_harness::ExoPreflightReport, ExoPreflightError> {
    let mut descriptor = descriptor;
    descriptor.identity = trusted.identity.clone();
    preflight(&descriptor, trusted)
}

pub fn pad_frame(frame: &[u8], limit: usize) -> Vec<u8> {
    assert!(frame.len() < limit);
    let mut padded = frame.to_vec();
    padded.resize(limit, b' ');
    padded
}

pub fn enable_minimum_capabilities(descriptor: &mut ExoCapabilityDescriptor) {
    descriptor.evidence.terminal_decision = ExoCapabilityState::Supported;
    descriptor.evidence.turn_identity = ExoCapabilityState::Supported;
    descriptor.lifecycle.graceful_eof = ExoCapabilityState::Supported;
    descriptor.lifecycle.idempotency = ExoCapabilityState::Supported;
    descriptor.lifecycle.cancellation = ExoCapabilityState::Supported;
    descriptor.lifecycle.recovery = ExoCapabilityState::Supported;
}
