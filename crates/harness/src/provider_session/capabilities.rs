// SPDX-License-Identifier: MIT

use super::common::{
    MAX_COMPLETED_TURNS, MAX_HISTORY_TTL_SECONDS, NATIVE_FRAME_SCHEMA, SESSION_CAPABILITIES_SCHEMA,
    SESSION_POLICY_SCHEMA, SessionError, digest, valid_digest, valid_id, valid_method,
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
    pub strict_executable: bool,
    pub experimental_api: bool,
    pub unknown_methods: String,
    pub raw_rpc: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveSessionLimits {
    pub policy_schema: String,
    pub max_completed_turns: usize,
    pub max_history_ttl_seconds: u64,
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
        Self {
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
                max_completed_turns: MAX_COMPLETED_TURNS,
                max_history_ttl_seconds: MAX_HISTORY_TTL_SECONDS,
            },
            strict_executable: false,
            experimental_api: false,
            unknown_methods: "deny".to_owned(),
            raw_rpc: false,
        }
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
            || self.effective_limits.policy_schema != SESSION_POLICY_SCHEMA
            || self.effective_limits.max_completed_turns == 0
            || self.effective_limits.max_completed_turns > MAX_COMPLETED_TURNS
            || self.effective_limits.max_history_ttl_seconds == 0
            || self.effective_limits.max_history_ttl_seconds > MAX_HISTORY_TTL_SECONDS
            || self.unknown_methods != "deny"
            || self.raw_rpc
        {
            return Err(SessionError::InvalidCapabilities);
        }
        Ok(())
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
