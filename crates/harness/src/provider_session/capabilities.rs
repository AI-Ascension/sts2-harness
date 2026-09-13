// SPDX-License-Identifier: MIT

use super::common::{
    MAX_CANDIDATES, MAX_COMPLETED_TURNS, MAX_DEPENDENCIES, MAX_EVENTS, MAX_FRAME_BYTES,
    MAX_HISTORY_BYTES, MAX_HISTORY_TTL_SECONDS, MAX_JSON_DEPTH, MAX_MAINTENANCE_JOBS,
    MAX_METHOD_BYTES, MAX_OPERATIONS, MAX_OUTPUT_SCHEMA_BYTES, MAX_PREPARED, MAX_PREPARED_BYTES,
    MAX_SESSION_ITEMS, MAX_SUFFIX_BYTES, NATIVE_FRAME_SCHEMA, SESSION_CAPABILITIES_OWNER,
    SESSION_CAPABILITIES_REVISION, SESSION_CAPABILITIES_SCHEMA, SESSION_POLICY_SCHEMA,
    SessionError, digest, valid_digest, valid_id, valid_method,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityHardening {
    pub tools_enabled: bool,
    pub ambient_history: bool,
    pub encrypted_state: bool,
    pub configuration_verified: bool,
    pub transform_handling: TransformHandling,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCapabilities {
    pub schema: String,
    pub profile_id: String,
    pub profile_sha256: String,
    pub native_version: String,
    pub native_binary_sha256: String,
    pub native_schema_sha256: String,
    pub evidence: CapabilityEvidence,
    pub transport: String,
    pub enabled_methods: Vec<String>,
    pub hardening: CapabilityHardening,
    /// Limits the selected native profile can execute. They bind this capability descriptor to the
    /// policy revision and must be checked before a session is admitted.
    pub effective_limits: EffectiveSessionLimits,
    pub binding: SessionCapabilityBinding,
    pub strict_executable: bool,
    pub experimental_api: bool,
    pub unknown_methods: String,
    pub raw_rpc: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCapabilityBinding {
    pub owner: String,
    pub owner_revision: String,
    pub policy_schema_sha256: String,
    pub model_revision: String,
    pub adapter_revision: String,
    pub adapter_revision_sha256: String,
    pub descriptor_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveSessionLimits {
    pub policy_schema: String,
    pub max_session_items: usize,
    pub max_dependencies: usize,
    pub max_events: usize,
    pub max_operations: usize,
    pub max_prepared: usize,
    pub max_candidates: usize,
    pub max_maintenance_jobs: usize,
    pub max_completed_turns: usize,
    pub max_history_ttl_seconds: u64,
    pub max_frame_bytes: usize,
    pub max_history_bytes: usize,
    pub max_prepared_bytes: usize,
    pub max_suffix_bytes: usize,
    pub max_output_schema_bytes: usize,
    pub max_method_bytes: usize,
    pub max_json_depth: usize,
}

impl EffectiveSessionLimits {
    fn validate(&self) -> Result<(), SessionError> {
        if self.policy_schema != SESSION_POLICY_SCHEMA
            || self.max_session_items == 0
            || self.max_session_items > MAX_SESSION_ITEMS
            || self.max_dependencies == 0
            || self.max_dependencies > MAX_DEPENDENCIES
            || self.max_events == 0
            || self.max_events > MAX_EVENTS
            || self.max_operations == 0
            || self.max_operations > MAX_OPERATIONS
            || self.max_prepared == 0
            || self.max_prepared > MAX_PREPARED
            || self.max_candidates == 0
            || self.max_candidates > MAX_CANDIDATES
            || self.max_maintenance_jobs == 0
            || self.max_maintenance_jobs > MAX_MAINTENANCE_JOBS
            || self.max_completed_turns == 0
            || self.max_completed_turns > MAX_COMPLETED_TURNS
            || self.max_history_ttl_seconds == 0
            || self.max_history_ttl_seconds > MAX_HISTORY_TTL_SECONDS
            || self.max_frame_bytes == 0
            || self.max_frame_bytes > MAX_FRAME_BYTES
            || self.max_history_bytes == 0
            || self.max_history_bytes > MAX_HISTORY_BYTES
            || self.max_prepared_bytes == 0
            || self.max_prepared_bytes > MAX_PREPARED_BYTES
            || self.max_suffix_bytes == 0
            || self.max_suffix_bytes > MAX_SUFFIX_BYTES
            || self.max_output_schema_bytes == 0
            || self.max_output_schema_bytes > MAX_OUTPUT_SCHEMA_BYTES
            || self.max_method_bytes == 0
            || self.max_method_bytes > MAX_METHOD_BYTES
            || self.max_json_depth == 0
            || self.max_json_depth > MAX_JSON_DEPTH
        {
            return Err(SessionError::InvalidCapabilities);
        }
        Ok(())
    }
}

impl SessionCapabilityBinding {
    fn validate(&self, capabilities: &NativeCapabilities) -> Result<(), SessionError> {
        if self.owner != SESSION_CAPABILITIES_OWNER
            || self.owner_revision != SESSION_CAPABILITIES_REVISION
            || self.policy_schema_sha256 != session_policy_schema_sha256()
            || !valid_id(&self.model_revision)
            || self.model_revision != capabilities.native_version
            || !valid_id(&self.adapter_revision)
            || self.adapter_revision != capabilities.profile_id
            || self.adapter_revision_sha256 != capabilities.profile_sha256
            || !valid_digest(&self.descriptor_sha256)
        {
            return Err(SessionError::InvalidCapabilities);
        }
        Ok(())
    }
}

#[must_use]
pub fn session_policy_schema_sha256() -> String {
    crate::sha256_hex(include_bytes!(
        "../../../../contracts/provider-session/policy.schema.json"
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEvidence {
    SchemaOnly,
    CompiledPeer,
    NativeBinaryFakeUpstream,
    LiveProvider,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransformHandling {
    VerifiedSuppressed,
    DetectAndFence,
    OpaqueApproved,
}

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
            evidence: CapabilityEvidence::CompiledPeer,
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

    pub fn validate(&self) -> Result<(), SessionError> {
        if self.schema != SESSION_CAPABILITIES_SCHEMA
            || !valid_id(&self.profile_id)
            || !valid_digest(&self.profile_sha256)
            || !valid_id(&self.native_version)
            || !valid_digest(&self.native_binary_sha256)
            || !valid_digest(&self.native_schema_sha256)
            || self.transport != "owned_stdio"
            || self.enabled_methods.is_empty()
            || self.enabled_methods.len() > 32
            || self
                .enabled_methods
                .iter()
                .any(|v| !valid_method(v) || !allowlisted_method(v))
            || self.enabled_methods.iter().collect::<BTreeSet<_>>().len()
                != self.enabled_methods.len()
            || self.hardening.tools_enabled
            || self.hardening.ambient_history
            || self.unknown_methods != "deny"
            || self.raw_rpc
        {
            return Err(SessionError::InvalidCapabilities);
        }
        self.effective_limits.validate()?;
        self.binding.validate(self)?;
        if self.binding.descriptor_sha256 != self.descriptor_digest() {
            return Err(SessionError::InvalidCapabilities);
        }
        Ok(())
    }

    #[must_use]
    pub fn descriptor_digest(&self) -> String {
        let mut unsigned = self.clone();
        unsigned.binding.descriptor_sha256.clear();
        crate::sha256_hex(serde_json::to_vec(&unsigned).unwrap_or_default())
    }
}

fn allowlisted_method(value: &str) -> bool {
    matches!(
        value,
        "initialize"
            | "thread/start"
            | "thread/read"
            | "turn/start"
            | "turn/interrupt"
            | "thread/fork"
            | "thread/compact/start"
    )
}
