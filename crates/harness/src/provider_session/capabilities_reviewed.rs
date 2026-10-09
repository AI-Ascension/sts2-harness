// SPDX-License-Identifier: MIT

use crate::ExoIdentity;

impl NativeCapabilities {
    #[must_use]
    pub fn fixture() -> Self {
        let methods = [
            "initialize",
            "thread/start",
            "thread/read",
            "turn/start",
            "turn/interrupt",
            "thread/fork",
            "thread/compact/start",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let mut capabilities = Self {
            schema: SESSION_CAPABILITIES_SCHEMA.to_owned(),
            profile_id: "codex-app-server-fixture-v1".to_owned(),
            profile_sha256: digest(b"codex-app-server-fixture-v1"),
            native_version: "fixture-peer-1".to_owned(),
            native_binary_sha256: digest(b"compiled-fake-native-peer"),
            native_schema_sha256: digest(NATIVE_FRAME_SCHEMA.as_bytes()),
            provenance: CapabilityProvenance::CompiledPeer,
            transport: "owned_stdio".to_owned(),
            enabled_methods: methods,
            hardening: CapabilityHardening {
                tools_enabled: false,
                ambient_history: false,
                encrypted_state: false,
                configuration_verified: true,
                transform_handling: TransformHandling::DetectAndFence,
            },
            effective_limits: EffectiveSessionLimits {
                policy_schema: SESSION_POLICY_SCHEMA.to_owned(),
                max_session_items: MAX_SESSION_ITEMS,
                max_dependencies: MAX_DEPENDENCIES,
                max_events: MAX_EVENTS,
                max_operations: MAX_OPERATIONS,
                max_prepared: MAX_PREPARED,
                max_candidates: MAX_CANDIDATES,
                max_maintenance_jobs: MAX_MAINTENANCE_JOBS,
                max_completed_turns: MAX_COMPLETED_TURNS,
                max_history_ttl_seconds: MAX_HISTORY_TTL_SECONDS,
                max_frame_bytes: MAX_FRAME_BYTES,
                max_history_bytes: MAX_HISTORY_BYTES,
                max_prepared_bytes: MAX_PREPARED_BYTES,
                max_suffix_bytes: MAX_SUFFIX_BYTES,
                max_output_schema_bytes: MAX_OUTPUT_SCHEMA_BYTES,
                max_method_bytes: MAX_METHOD_BYTES,
                max_json_depth: MAX_JSON_DEPTH,
            },
            binding: SessionCapabilityBinding {
                owner: SESSION_CAPABILITIES_OWNER.to_owned(),
                owner_revision: SESSION_CAPABILITIES_REVISION.to_owned(),
                policy_schema_sha256: session_policy_schema_sha256(),
                model_revision: "fixture-peer-1".to_owned(),
                adapter_revision: "codex-app-server-fixture-v1".to_owned(),
                adapter_revision_sha256: digest(b"codex-app-server-fixture-v1"),
                descriptor_sha256: String::new(),
            },
            strict_executable: false,
            experimental_api: false,
            unknown_methods: "deny".to_owned(),
            raw_rpc: false,
        };
        capabilities.binding.descriptor_sha256 = capabilities.descriptor_digest();
        capabilities
    }

    /// Builds the narrow capability descriptor for the harness-owned Exo lifecycle v2 adapter.
    ///
    /// This is deliberately a source-derived profile rather than a claim about an upstream
    /// Codex peer: the adapter launches one owned process per turn and maps only its verified
    /// operations.  It therefore cannot advertise read, fork, compaction, reuse, or raw RPC.
    /// `executor_sha256`, `wire_schema_sha256`, and `profile_sha256` must come from the inspected
    /// launch configuration; callers cannot use a policy declaration as their source.
    pub fn reviewed_exo_lifecycle(
        profile_id: &str,
        profile_sha256: String,
        executor_sha256: String,
        wire_schema_sha256: String,
    ) -> Result<Self, SessionError> {
        if !valid_id(profile_id)
            || !valid_digest(&profile_sha256)
            || !valid_digest(&executor_sha256)
            || !valid_digest(&wire_schema_sha256)
        {
            return Err(SessionError::InvalidCapabilities);
        }
        let mut capabilities = Self::fixture();
        capabilities.profile_id = profile_id.to_owned();
        capabilities.profile_sha256 = profile_sha256.clone();
        capabilities.native_version = profile_id.to_owned();
        capabilities.native_binary_sha256 = executor_sha256;
        capabilities.native_schema_sha256 = wire_schema_sha256;
        capabilities.provenance = CapabilityProvenance::SchemaOnly;
        capabilities.enabled_methods = vec![
            String::from("initialize"),
            String::from("turn/start"),
            String::from("turn/interrupt"),
        ];
        capabilities.binding.model_revision = profile_id.to_owned();
        capabilities.binding.adapter_revision = profile_id.to_owned();
        capabilities.binding.adapter_revision_sha256 = profile_sha256;
        capabilities.binding.descriptor_sha256 = capabilities.descriptor_digest();
        capabilities.validate()?;
        Ok(capabilities)
    }
}

impl NativeCapabilities {
    /// Describes an opaque guarded synthetic inspection as schema-only metadata.
    pub fn reviewed_synthetic_exo_one_shot(
        inspection: &crate::exo_bridge_configuration::SyntheticLoopbackInspection,
    ) -> Result<Self, SessionError> {
        synthetic_exo_one_shot(inspection.inspected_identity())
    }

    /// Rebuilds the schema-only descriptor from an opaque synthetic plan.
    pub fn reviewed_synthetic_exo_one_shot_from_plan(
        plan: &crate::SyntheticExoAdmissionPlan,
    ) -> Result<Self, SessionError> {
        synthetic_exo_one_shot(plan.inspected_identity())
    }
}

const SYNTHETIC_PROFILE_DIGEST_DOMAIN: &str =
    "sts2-harness-exo-synthetic-one-shot-provider-profile-v1";
const SYNTHETIC_PROVIDER_PROFILE_ID: &str = "sts2-exo-synthetic-one-shot-v1";
const SYNTHETIC_PROVIDER_PROFILE_VERSION: &str = "synthetic-envelope-one-shot-v1";
const SYNTHETIC_INFERENCE_PROFILE_ADAPTER: &str = "exo.runtime-v3";
const SYNTHETIC_RUNTIME_SELECTOR: &str = "runtime-v3-gameplay";
const SYNTHETIC_PROVIDER_METHOD: &str = "turn/start";
const SYNTHETIC_GUARDED_CONFIGURATION_SCHEMA: &str = "sts2.exo-one-shot-config-v2";
const SYNTHETIC_EXECUTOR_INPUT_SCHEMA: &str = "sts2.exo-executor-input-v2";
const SYNTHETIC_EXECUTOR_RECEIPT_SCHEMA: &str = "sts2.exo-executor-receipt-v2";
const SYNTHETIC_EXO_BRIDGE_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../protocol-artifact/exo-bridge-v1/schema.json");

fn synthetic_exo_one_shot(identity: &ExoIdentity) -> Result<NativeCapabilities, SessionError> {
    identity
        .validate_synthetic_loopback()
        .map_err(|_| SessionError::InvalidCapabilities)?;
    if !identity.is_complete()
        || identity.source_revision != crate::EXO_SOURCE_REVISION
        || identity.contract_version != crate::EXO_CONTRACT_VERSION
        || identity.provider.as_deref() != Some("openai")
    {
        return Err(SessionError::InvalidCapabilities);
    }

    let schema_sha256 = crate::sha256_hex(SYNTHETIC_EXO_BRIDGE_SCHEMA_BYTES);
    let digest_input = serde_json::json!({
        "domain": SYNTHETIC_PROFILE_DIGEST_DOMAIN,
        "provider_session_schema": SESSION_CAPABILITIES_SCHEMA,
        "profile_version": SYNTHETIC_PROVIDER_PROFILE_VERSION,
        "runtime_selector": SYNTHETIC_RUNTIME_SELECTOR,
        "inference_profile_adapter": SYNTHETIC_INFERENCE_PROFILE_ADAPTER,
        "provider_method": SYNTHETIC_PROVIDER_METHOD,
        "exo_contract_version": crate::EXO_CONTRACT_VERSION,
        "identity": identity,
        "outer_wire": crate::EXO_BRIDGE_WIRE_VERSION,
        "decision_schema": crate::EXO_DECISION_SCHEMA,
        "guarded_configuration_schema": SYNTHETIC_GUARDED_CONFIGURATION_SCHEMA,
        "executor_input_schema": SYNTHETIC_EXECUTOR_INPUT_SCHEMA,
        "executor_receipt_schema": SYNTHETIC_EXECUTOR_RECEIPT_SCHEMA,
        "native_schema_sha256": schema_sha256,
    });
    let encoded =
        serde_json::to_vec(&digest_input).map_err(|_| SessionError::InvalidCapabilities)?;
    let profile_sha256 = crate::sha256_hex(encoded);
    let model = identity
        .model_binding
        .as_deref()
        .ok_or(SessionError::InvalidCapabilities)?;
    let package_digest = identity
        .package_digest
        .as_deref()
        .ok_or(SessionError::InvalidCapabilities)?;

    let mut capabilities = NativeCapabilities::fixture();
    capabilities.profile_id = SYNTHETIC_PROVIDER_PROFILE_ID.to_owned();
    capabilities.profile_sha256 = profile_sha256.clone();
    capabilities.native_version = model.to_owned();
    capabilities.native_binary_sha256 = package_digest.to_owned();
    capabilities.native_schema_sha256 = schema_sha256;
    capabilities.provenance = CapabilityProvenance::SchemaOnly;
    capabilities.enabled_methods = vec![SYNTHETIC_PROVIDER_METHOD.to_owned()];
    capabilities.binding.model_revision = model.to_owned();
    capabilities.binding.adapter_revision = SYNTHETIC_PROVIDER_PROFILE_ID.to_owned();
    capabilities.binding.adapter_revision_sha256 = profile_sha256;
    capabilities.binding.descriptor_sha256 = capabilities.descriptor_digest();
    capabilities.validate()?;
    Ok(capabilities)
}
