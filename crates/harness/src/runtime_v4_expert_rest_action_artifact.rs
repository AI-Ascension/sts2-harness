// SPDX-License-Identifier: MIT

use serde_json::Value;

pub const RUNTIME_V4_EXPERT_REST_ACTION_PROTOCOL_VERSION: &str = "runtime-v4-expert-rest-action-v1";
pub const RUNTIME_V4_EXPERT_REST_ACTION_ARTIFACT: &str =
    "sts2-protocol/runtime-v4-expert-rest-action";
pub const RUNTIME_V4_EXPERT_REST_ACTION_SCHEMA_SOURCE: &str =
    "schemas/runtime-v4-expert-rest-action-v1.schema.json";
pub const RUNTIME_V4_EXPERT_REST_ACTION_GENERATOR: &str = "hand-authored";
pub const RUNTIME_V4_EXPERT_REST_ACTION_SCHEMA_DIGEST: &str =
    "bb3555fae28eb1f79d08a15e9884696a579e4c20836f5016509f17e0f4c36fbd";

const MANIFEST_BYTES: &[u8] =
    include_bytes!("../../../protocol-artifact/runtime-v4-expert-rest-action/manifest.json");
const SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../protocol-artifact/runtime-v4-expert-rest-action/schema.json");
const SOURCE_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../schemas/runtime-v4-expert-rest-action-v1.schema.json");
const CONFORMANCE_BYTES: &[u8] =
    include_bytes!("../../../conformance/cases/runtime-v4-expert-rest-action-v1.json");
const CHECKSUMS: &str =
    include_str!("../../../protocol-artifact/runtime-v4-expert-rest-action/SHA256SUMS");

include!("runtime_v4_expert_rest_action_checksums.rs");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeV4ExpertRestActionArtifactError {
    ManifestMismatch,
    SchemaMismatch,
    ConformanceMismatch,
    ChecksumMismatch,
}

impl std::fmt::Display for RuntimeV4ExpertRestActionArtifactError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ManifestMismatch => "runtime-v4-expert-rest-action manifest mismatch",
            Self::SchemaMismatch => "runtime-v4-expert-rest-action schema mismatch",
            Self::ConformanceMismatch => "runtime-v4-expert-rest-action conformance mismatch",
            Self::ChecksumMismatch => "runtime-v4-expert-rest-action checksum mismatch",
        })
    }
}

impl std::error::Error for RuntimeV4ExpertRestActionArtifactError {}

/// Verify every byte consumed by the harness from the pinned protocol artifact.
///
/// The artifact is currently a candidate owned by `sts2-protocol`; this check deliberately pins
/// its identity and checksums without changing the protocol manifest's admission status. A later
/// admitted artifact must be copied as a complete payload and will be reverified at that boundary.
pub fn verify_runtime_v4_expert_rest_action_artifact()
-> Result<(), RuntimeV4ExpertRestActionArtifactError> {
    let manifest: Value = serde_json::from_slice(MANIFEST_BYTES)
        .map_err(|_| RuntimeV4ExpertRestActionArtifactError::ManifestMismatch)?;
    if !manifest_matches(&manifest) {
        return Err(RuntimeV4ExpertRestActionArtifactError::ManifestMismatch);
    }
    let schema: Value = serde_json::from_slice(SCHEMA_BYTES)
        .map_err(|_| RuntimeV4ExpertRestActionArtifactError::SchemaMismatch)?;
    if SCHEMA_BYTES != SOURCE_SCHEMA_BYTES
        || schema.get("$id").and_then(Value::as_str)
            != Some("sts2-runtime-v4-expert-rest-action-v1")
    {
        return Err(RuntimeV4ExpertRestActionArtifactError::SchemaMismatch);
    }
    let conformance: Value = serde_json::from_slice(CONFORMANCE_BYTES)
        .map_err(|_| RuntimeV4ExpertRestActionArtifactError::ConformanceMismatch)?;
    if conformance.get("contract").and_then(Value::as_str)
        != Some("sts2.protocol/runtime-v4-expert-rest-action-v1")
    {
        return Err(RuntimeV4ExpertRestActionArtifactError::ConformanceMismatch);
    }
    if !checksums_match() {
        return Err(RuntimeV4ExpertRestActionArtifactError::ChecksumMismatch);
    }
    Ok(())
}

fn manifest_matches(manifest: &Value) -> bool {
    let Some(object) = manifest.as_object() else {
        return false;
    };
    object.get("artifact").and_then(Value::as_str) == Some(RUNTIME_V4_EXPERT_REST_ACTION_ARTIFACT)
        && object.get("protocol_version").and_then(Value::as_str)
            == Some(RUNTIME_V4_EXPERT_REST_ACTION_PROTOCOL_VERSION)
        && object.get("schema").and_then(Value::as_str) == Some("schema.json")
        && object.get("schema_digest").and_then(Value::as_str)
            == Some(RUNTIME_V4_EXPERT_REST_ACTION_SCHEMA_DIGEST)
        && object.get("checksums").and_then(Value::as_str) == Some("SHA256SUMS")
        && object
            .get("provenance")
            .and_then(Value::as_object)
            .is_some_and(|provenance| {
                provenance.get("source").and_then(Value::as_str)
                    == Some(RUNTIME_V4_EXPERT_REST_ACTION_SCHEMA_SOURCE)
                    && provenance.get("generator").and_then(Value::as_str)
                        == Some(RUNTIME_V4_EXPERT_REST_ACTION_GENERATOR)
                    && provenance.get("license").and_then(Value::as_str) == Some("MIT")
            })
}

fn checksums_match() -> bool {
    let mut seen = vec![false; CHECKSUM_FILES.len()];
    let mut listed = 0;
    for line in CHECKSUMS.lines() {
        let Some((digest, path)) = line.split_once("  ") else {
            return false;
        };
        let Some(index) = CHECKSUM_FILES.iter().position(|(known, _)| *known == path) else {
            return false;
        };
        if seen[index]
            || digest.len() != 64
            || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            || crate::sha256_hex(CHECKSUM_FILES[index].1) != digest
        {
            return false;
        }
        seen[index] = true;
        listed += 1;
    }
    listed == CHECKSUM_FILES.len() && seen.into_iter().all(|value| value)
}

#[cfg(test)]
mod tests {
    use super::verify_runtime_v4_expert_rest_action_artifact;

    #[test]
    fn pinned_rest_action_payload_is_self_consistent() -> Result<(), Box<dyn std::error::Error>> {
        verify_runtime_v4_expert_rest_action_artifact()?;
        Ok(())
    }
}
