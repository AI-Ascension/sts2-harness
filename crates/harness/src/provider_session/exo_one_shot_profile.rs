// SPDX-License-Identifier: MIT

use super::{CapabilityProvenance, NativeCapabilities, SESSION_CAPABILITIES_SCHEMA, SessionError};
use crate::exo::{
    EXO_BRIDGE_WIRE_VERSION, EXO_CONTRACT_VERSION, EXO_DECISION_SCHEMA, EXO_SOURCE_REVISION,
    ExoCapabilityDescriptor, ExoIdentity, responses_capable, responses_routing_capable,
};
use serde::Serialize;

const PROFILE_DIGEST_DOMAIN: &str = "sts2-harness-exo-one-shot-provider-profile-v1";
const PROVIDER_PROFILE_VERSION: &str = "ordinary-one-shot-v1";
const INFERENCE_PROFILE_ADAPTER: &str = "exo.runtime-v3";
const RUNTIME_SELECTOR: &str = "runtime-v3-gameplay";
const PROVIDER_METHOD: &str = "turn/start";
const GUARDED_CONFIGURATION_SCHEMA: &str = "sts2.exo-one-shot-config-v2";
const EXECUTOR_INPUT_SCHEMA: &str = "sts2.exo-executor-input-v2";
const EXECUTOR_RECEIPT_SCHEMA: &str = "sts2.exo-executor-receipt-v2";
const EXO_BRIDGE_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../protocol-artifact/exo-bridge-v1/schema.json");

#[derive(Serialize)]
struct ProfileDigestInput<'a> {
    domain: &'static str,
    provider_session_schema: &'static str,
    profile_version: &'static str,
    runtime_selector: &'static str,
    inference_profile_adapter: &'static str,
    provider_method: &'static str,
    exo_contract_version: &'static str,
    identity: &'a ExoIdentity,
    outer_wire: &'static str,
    decision_schema: &'static str,
    guarded_configuration_schema: &'static str,
    executor_input_schema: &'static str,
    executor_receipt_schema: &'static str,
    native_schema_sha256: &'a str,
}

impl NativeCapabilities {
    /// Describes the reviewed ordinary Exo one-shot profile from a complete deployment identity.
    ///
    /// The identity must come from the guarded loader's `Loaded::inspected_identity` path at
    /// runtime boundaries. This pure builder cannot establish that provenance itself: callers
    /// must compare the result against the identity independently inspected by the served runtime.
    /// The descriptor is schema-only and does not admit the provider-session broker or claim a
    /// provider call. The sole proposed method is one `turn/start` mapping to an ordinary decision.
    pub fn reviewed_exo_one_shot(identity: &ExoIdentity) -> Result<Self, SessionError> {
        validate_identity(identity)?;

        let schema_sha256 = crate::sha256_hex(EXO_BRIDGE_SCHEMA_BYTES);
        let profile_sha256 = profile_sha256(identity, &schema_sha256)?;
        let model = identity
            .model_binding
            .as_deref()
            .ok_or(SessionError::InvalidCapabilities)?;
        let package_digest = identity
            .package_digest
            .as_deref()
            .ok_or(SessionError::InvalidCapabilities)?;

        let mut capabilities = Self::fixture();
        capabilities.profile_id = identity.source_revision.clone();
        capabilities.profile_sha256 = profile_sha256.clone();
        capabilities.native_version = model.to_owned();
        capabilities.native_binary_sha256 = package_digest.to_owned();
        capabilities.native_schema_sha256 = schema_sha256;
        capabilities.provenance = CapabilityProvenance::SchemaOnly;
        capabilities.enabled_methods = vec![PROVIDER_METHOD.to_owned()];
        capabilities.binding.model_revision = model.to_owned();
        capabilities.binding.adapter_revision = identity.source_revision.clone();
        capabilities.binding.adapter_revision_sha256 = profile_sha256;
        capabilities.binding.descriptor_sha256.clear();
        capabilities.binding.descriptor_sha256 = capabilities.descriptor_digest();
        capabilities.validate()?;
        Ok(capabilities)
    }
}

fn validate_identity(identity: &ExoIdentity) -> Result<(), SessionError> {
    let mut descriptor =
        ExoCapabilityDescriptor::source_review().map_err(|_| SessionError::InvalidCapabilities)?;
    descriptor.identity = identity.clone();
    descriptor
        .validate()
        .map_err(|_| SessionError::InvalidCapabilities)?;

    if !identity.is_complete()
        || identity.source_revision != EXO_SOURCE_REVISION
        || identity.contract_version != EXO_CONTRACT_VERSION
    {
        return Err(SessionError::InvalidCapabilities);
    }
    let provider = identity
        .provider
        .as_deref()
        .ok_or(SessionError::InvalidCapabilities)?;
    let endpoint = identity
        .endpoint
        .as_deref()
        .ok_or(SessionError::InvalidCapabilities)?;
    let model = identity
        .model_binding
        .as_deref()
        .ok_or(SessionError::InvalidCapabilities)?;
    if !responses_routing_capable(provider, endpoint) || !responses_capable(model) {
        return Err(SessionError::InvalidCapabilities);
    }
    Ok(())
}

fn profile_sha256(identity: &ExoIdentity, schema_sha256: &str) -> Result<String, SessionError> {
    let input = ProfileDigestInput {
        domain: PROFILE_DIGEST_DOMAIN,
        provider_session_schema: SESSION_CAPABILITIES_SCHEMA,
        profile_version: PROVIDER_PROFILE_VERSION,
        runtime_selector: RUNTIME_SELECTOR,
        inference_profile_adapter: INFERENCE_PROFILE_ADAPTER,
        provider_method: PROVIDER_METHOD,
        exo_contract_version: EXO_CONTRACT_VERSION,
        identity,
        outer_wire: EXO_BRIDGE_WIRE_VERSION,
        decision_schema: EXO_DECISION_SCHEMA,
        guarded_configuration_schema: GUARDED_CONFIGURATION_SCHEMA,
        executor_input_schema: EXECUTOR_INPUT_SCHEMA,
        executor_receipt_schema: EXECUTOR_RECEIPT_SCHEMA,
        native_schema_sha256: schema_sha256,
    };
    let bytes = serde_json::to_vec(&input).map_err(|_| SessionError::InvalidCapabilities)?;
    Ok(crate::sha256_hex(bytes))
}

#[cfg(test)]
#[path = "exo_one_shot_profile_tests.rs"]
mod tests;
