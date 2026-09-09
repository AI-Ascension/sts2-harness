// SPDX-License-Identifier: MIT

use serde_json::Value;
use sha2::{Digest, Sha256};

pub const RUNTIME_V4_EXPERT_ACTION_PROTOCOL_VERSION: &str = "runtime-v4-expert-action";
pub const RUNTIME_V4_EXPERT_ACTION_ARTIFACT: &str = "sts2-protocol/runtime-v4-expert-action";
pub const RUNTIME_V4_EXPERT_ACTION_SCHEMA_SOURCE: &str =
    "schemas/runtime-v4-expert-action.schema.json";
pub const RUNTIME_V4_EXPERT_ACTION_GENERATOR: &str = "hand-authored";
pub const RUNTIME_V4_EXPERT_ACTION_SCHEMA_DIGEST: &str =
    "393318bda8c3522c0ecbacc78b95471a9f4dc3f825169d2048f4c74a7b7f2929";

const MANIFEST_BYTES: &[u8] =
    include_bytes!("../../../protocol-artifact/runtime-v4-expert-action/manifest.json");
const SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../protocol-artifact/runtime-v4-expert-action/schema.json");
const SOURCE_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../schemas/runtime-v4-expert-action.schema.json");
const CONFORMANCE_BYTES: &[u8] =
    include_bytes!("../../../conformance/cases/runtime-v4-expert-action.json");
const REQUEST_BYTES: &[u8] = include_bytes!(
    "../../../protocol-artifact/runtime-v4-expert-action/golden/action-request.json"
);
const SETTLED_BYTES: &[u8] = include_bytes!(
    "../../../protocol-artifact/runtime-v4-expert-action/golden/action-settled.json"
);
const CHECKSUMS: &str =
    include_str!("../../../protocol-artifact/runtime-v4-expert-action/SHA256SUMS");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeV4ExpertActionArtifactError {
    ManifestMismatch,
    SchemaMismatch,
    ConformanceMismatch,
    GoldenMismatch,
    ChecksumMismatch,
}

impl std::fmt::Display for RuntimeV4ExpertActionArtifactError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ManifestMismatch => "runtime-v4-expert-action manifest mismatch",
            Self::SchemaMismatch => "runtime-v4-expert-action schema mismatch",
            Self::ConformanceMismatch => "runtime-v4-expert-action conformance mismatch",
            Self::GoldenMismatch => "runtime-v4-expert-action golden mismatch",
            Self::ChecksumMismatch => "runtime-v4-expert-action checksum mismatch",
        })
    }
}

impl std::error::Error for RuntimeV4ExpertActionArtifactError {}

pub fn verify_runtime_v4_expert_action_artifact() -> Result<(), RuntimeV4ExpertActionArtifactError>
{
    let manifest: Value = serde_json::from_slice(MANIFEST_BYTES)
        .map_err(|_| RuntimeV4ExpertActionArtifactError::ManifestMismatch)?;
    if !manifest_matches(&manifest) {
        return Err(RuntimeV4ExpertActionArtifactError::ManifestMismatch);
    }
    let schema: Value = serde_json::from_slice(SCHEMA_BYTES)
        .map_err(|_| RuntimeV4ExpertActionArtifactError::SchemaMismatch)?;
    if SCHEMA_BYTES != SOURCE_SCHEMA_BYTES
        || schema.get("$id").and_then(Value::as_str) != Some("sts2-runtime-v4-expert-action")
    {
        return Err(RuntimeV4ExpertActionArtifactError::SchemaMismatch);
    }
    let conformance: Value = serde_json::from_slice(CONFORMANCE_BYTES)
        .map_err(|_| RuntimeV4ExpertActionArtifactError::ConformanceMismatch)?;
    if conformance.get("contract").and_then(Value::as_str)
        != Some("sts2.protocol/runtime-v4-expert-action")
    {
        return Err(RuntimeV4ExpertActionArtifactError::ConformanceMismatch);
    }
    let request: Value = serde_json::from_slice(REQUEST_BYTES)
        .map_err(|_| RuntimeV4ExpertActionArtifactError::GoldenMismatch)?;
    let settled: Value = serde_json::from_slice(SETTLED_BYTES)
        .map_err(|_| RuntimeV4ExpertActionArtifactError::GoldenMismatch)?;
    if request.get("kind").and_then(Value::as_str) != Some("action_request")
        || settled.get("kind").and_then(Value::as_str) != Some("action_response")
        || request.get("schema_digest").and_then(Value::as_str)
            != Some(RUNTIME_V4_EXPERT_ACTION_SCHEMA_DIGEST)
        || settled.get("schema_digest").and_then(Value::as_str)
            != Some(RUNTIME_V4_EXPERT_ACTION_SCHEMA_DIGEST)
    {
        return Err(RuntimeV4ExpertActionArtifactError::GoldenMismatch);
    }
    if !checksums_match() {
        return Err(RuntimeV4ExpertActionArtifactError::ChecksumMismatch);
    }
    Ok(())
}

fn manifest_matches(manifest: &Value) -> bool {
    let Some(object) = manifest.as_object() else {
        return false;
    };
    object.len() == 8
        && object.get("artifact").and_then(Value::as_str) == Some(RUNTIME_V4_EXPERT_ACTION_ARTIFACT)
        && object.get("protocol_version").and_then(Value::as_str)
            == Some(RUNTIME_V4_EXPERT_ACTION_PROTOCOL_VERSION)
        && object.get("schema").and_then(Value::as_str) == Some("schema.json")
        && object.get("schema_digest").and_then(Value::as_str)
            == Some(RUNTIME_V4_EXPERT_ACTION_SCHEMA_DIGEST)
        && object.get("checksums").and_then(Value::as_str) == Some("SHA256SUMS")
        && object
            .get("provenance")
            .and_then(Value::as_object)
            .is_some_and(|provenance| {
                provenance.len() == 3
                    && provenance.get("source").and_then(Value::as_str)
                        == Some(RUNTIME_V4_EXPERT_ACTION_SCHEMA_SOURCE)
                    && provenance.get("generator").and_then(Value::as_str)
                        == Some(RUNTIME_V4_EXPERT_ACTION_GENERATOR)
                    && provenance.get("license").and_then(Value::as_str) == Some("MIT")
            })
        && object
            .get("consumers")
            .and_then(Value::as_array)
            .is_some_and(|consumers| {
                consumers
                    == &[
                        Value::from("sts2-game-mod"),
                        Value::from("sts2-gateway"),
                        Value::from("sts2-harness"),
                        Value::from("sts2-mcp-server"),
                    ]
            })
        && object
            .get("goldens")
            .and_then(Value::as_array)
            .is_some_and(|goldens| {
                goldens
                    == &[
                        Value::from("golden/action-request.json"),
                        Value::from("golden/action-settled.json"),
                    ]
            })
}

fn checksums_match() -> bool {
    let expected = [
        (
            "../../conformance/cases/runtime-v4-expert-action.json",
            CONFORMANCE_BYTES,
        ),
        (
            "../../schemas/runtime-v4-expert-action.schema.json",
            SOURCE_SCHEMA_BYTES,
        ),
        ("manifest.json", MANIFEST_BYTES),
        ("schema.json", SCHEMA_BYTES),
        ("golden/action-request.json", REQUEST_BYTES),
        ("golden/action-settled.json", SETTLED_BYTES),
    ];
    let mut seen = [false; 6];
    let mut listed = 0;
    for line in CHECKSUMS.lines() {
        let Some((digest, path)) = line.split_once("  ") else {
            return false;
        };
        let Some(index) = expected.iter().position(|(known, _)| *known == path) else {
            return false;
        };
        if seen[index]
            || digest.len() != 64
            || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            || format!("{:x}", Sha256::digest(expected[index].1)) != digest
        {
            return false;
        }
        seen[index] = true;
        listed += 1;
    }
    listed == expected.len() && seen.into_iter().all(|value| value)
}
