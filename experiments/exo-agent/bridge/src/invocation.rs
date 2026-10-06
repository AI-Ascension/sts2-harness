// SPDX-License-Identifier: MIT
//! Closed executor invocation schemas and version selection.

use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;

use super::{lookup_wire, private_state};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InvocationV1 {
    version: String,
    request_id: String,
    host_turn_id: String,
    model: String,
    endpoint: String,
    module_path: PathBuf,
    source_root: PathBuf,
    state_root: PathBuf,
    input: Value,
    timeout_millis: u32,
    max_output_tokens: u32,
    credential: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InvocationV2 {
    version: String,
    request_id: String,
    host_turn_id: String,
    model: String,
    endpoint: String,
    module_path: PathBuf,
    source_root: PathBuf,
    state_root: PathBuf,
    input: Value,
    timeout_millis: u32,
    max_output_tokens: u32,
    credential: String,
    private_state: private_state::PrivateState,
}

#[derive(Deserialize)]
struct InvocationVersion {
    version: String,
}

/// Private internal handoff from sts2-exo-bridge, never a model-visible control envelope.
pub(crate) struct Invocation {
    pub(crate) version: String,
    pub(crate) request_id: String,
    pub(crate) host_turn_id: String,
    pub(crate) model: String,
    pub(crate) endpoint: String,
    pub(crate) module_path: PathBuf,
    pub(crate) source_root: PathBuf,
    pub(crate) state_root: PathBuf,
    pub(crate) input: Value,
    pub(crate) timeout_millis: u32,
    pub(crate) max_output_tokens: u32,
    pub(crate) credential: String,
    pub(crate) private_state: Option<private_state::PrivateState>,
}

impl From<InvocationV1> for Invocation {
    fn from(value: InvocationV1) -> Self {
        Self {
            version: value.version,
            request_id: value.request_id,
            host_turn_id: value.host_turn_id,
            model: value.model,
            endpoint: value.endpoint,
            module_path: value.module_path,
            source_root: value.source_root,
            state_root: value.state_root,
            input: value.input,
            timeout_millis: value.timeout_millis,
            max_output_tokens: value.max_output_tokens,
            credential: value.credential,
            private_state: None,
        }
    }
}

impl From<InvocationV2> for Invocation {
    fn from(value: InvocationV2) -> Self {
        Self {
            version: value.version,
            request_id: value.request_id,
            host_turn_id: value.host_turn_id,
            model: value.model,
            endpoint: value.endpoint,
            module_path: value.module_path,
            source_root: value.source_root,
            state_root: value.state_root,
            input: value.input,
            timeout_millis: value.timeout_millis,
            max_output_tokens: value.max_output_tokens,
            credential: value.credential,
            private_state: Some(value.private_state),
        }
    }
}

pub(crate) fn decode_invocation(bytes: &[u8], lookup: bool) -> Result<Invocation, &'static str> {
    if lookup {
        let header: InvocationVersion = lookup_wire::decode(bytes)?;
        if !matches!(
            header.version.as_str(),
            "sts2.exo-lookup-executor-input-v1" | "sts2.exo-lookup-executor-input-v2"
        ) {
            return Err("exo_lookup_invocation");
        }
        if header.version.ends_with("-v2") {
            lookup_wire::decode::<InvocationV2>(bytes).map(Invocation::from)
        } else {
            lookup_wire::decode::<InvocationV1>(bytes).map(Invocation::from)
        }
    } else {
        let header: InvocationVersion =
            lookup_wire::decode(bytes).map_err(|_| "exo_executor_input_shape")?;
        if !matches!(
            header.version.as_str(),
            "sts2.exo-executor-input-v1" | "sts2.exo-executor-input-v2"
        ) {
            return Err("exo_executor_input_invalid");
        }
        if header.version.ends_with("-v2") {
            lookup_wire::decode::<InvocationV2>(bytes)
                .map(Invocation::from)
                .map_err(|_| "exo_executor_input_shape")
        } else {
            lookup_wire::decode::<InvocationV1>(bytes)
                .map(Invocation::from)
                .map_err(|_| "exo_executor_input_shape")
        }
    }
}

#[cfg(test)]
mod invocation_tests {
    use super::decode_invocation;
    use serde_json::{Value, json};

    fn common(version: &str) -> Value {
        json!({
            "version": version,
            "request_id": "request-1",
            "host_turn_id": "turn-1",
            "model": "o3-pro",
            "endpoint": "https://api.openai.com/v1",
            "module_path": "/opt/sts2/extension.js",
            "source_root": "/opt/sts2/source",
            "state_root": "/var/lib/sts2/state/0123456789abcdef0123456789abcdef",
            "input": {"observation": {}, "legal_action_ids": []},
            "timeout_millis": 1000,
            "max_output_tokens": 100,
            "credential": "secret"
        })
    }

    fn private_state() -> Value {
        json!({
            "version": "sts2.exo-private-state-executor-v2",
            "attempt_id": "0123456789abcdef0123456789abcdef",
            "config_digest": "a".repeat(64),
            "policy_digest": "b".repeat(64),
            "policy": {
                "state_root": "/var/lib/sts2/state",
                "cache_root": "/var/lib/sts2/cache",
                "temp_root": "/var/lib/sts2/temp",
                "quota_bytes": 1048576,
                "max_retention_days": 1,
                "permissions_octal": 448
            },
            "service_uid": 1000,
            "process": {
                "boot_id": "01234567-89ab-cdef-0123-456789abcdef",
                "pid": 1234,
                "parent_pid": 123,
                "process_group": 1234,
                "session": 1234,
                "start_time_ticks": 5678,
                "uid": 1000
            },
            "paths": {
                "state_root": "/var/lib/sts2/state/0123456789abcdef0123456789abcdef",
                "cache_root": "/var/lib/sts2/cache/0123456789abcdef0123456789abcdef",
                "temp_root": "/var/lib/sts2/temp/0123456789abcdef0123456789abcdef",
                "config_root": "/var/lib/sts2/cache/0123456789abcdef0123456789abcdef/config"
            }
        })
    }

    #[test]
    fn closed_v1_input_rejects_private_state_and_v2_requires_it() -> Result<(), &'static str> {
        let v1 =
            serde_json::to_vec(&common("sts2.exo-executor-input-v1")).map_err(|_| "fixture")?;
        assert!(decode_invocation(&v1, false)?.private_state.is_none());

        let mut expanded_v1 = common("sts2.exo-executor-input-v1");
        expanded_v1["private_state"] = private_state();
        let bytes = serde_json::to_vec(&expanded_v1).map_err(|_| "fixture")?;
        assert!(decode_invocation(&bytes, false).is_err());

        let mut v2 = common("sts2.exo-executor-input-v2");
        v2["private_state"] = private_state();
        let bytes = serde_json::to_vec(&v2).map_err(|_| "fixture")?;
        assert!(decode_invocation(&bytes, false)?.private_state.is_some());

        v2["unexpected"] = json!(true);
        let bytes = serde_json::to_vec(&v2).map_err(|_| "fixture")?;
        assert!(decode_invocation(&bytes, false).is_err());
        Ok(())
    }

    #[test]
    fn lookup_input_uses_its_separate_closed_v2_name() -> Result<(), &'static str> {
        let mut lookup = common("sts2.exo-lookup-executor-input-v2");
        lookup["private_state"] = private_state();
        let bytes = serde_json::to_vec(&lookup).map_err(|_| "fixture")?;
        let invocation = decode_invocation(&bytes, true)?;
        assert_eq!(invocation.version, "sts2.exo-lookup-executor-input-v2");
        assert!(invocation.private_state.is_some());
        assert!(decode_invocation(&bytes, false).is_err());
        Ok(())
    }

    #[test]
    fn invocation_decoder_rejects_duplicate_members() {
        assert!(decode_invocation(
            br#"{"version":"sts2.exo-executor-input-v1","version":"sts2.exo-executor-input-v2"}"#,
            false
        )
        .is_err());
    }
}
