// SPDX-License-Identifier: MIT

use serde_json::Value;
use sha2::{Digest, Sha256};

/// Version consumed by the additive expert-state harness lane.
pub const RUNTIME_V4_EXPERT_PROTOCOL_VERSION: &str = "runtime-v4-expert";
/// Schema digest supplied by the protocol artifact.
pub const RUNTIME_V4_EXPERT_SCHEMA_DIGEST: &str =
    "f0786b039396043a441323447ac44f7cc4c218071bc477722f3ec992ab295a8a";
/// Release-like artifact identity, not a Cargo path dependency.
pub const RUNTIME_V4_EXPERT_ARTIFACT: &str = "sts2-protocol/runtime-v4-expert";
/// Normative source recorded in artifact provenance.
pub const RUNTIME_V4_EXPERT_SCHEMA_SOURCE: &str = "schemas/runtime-v4-expert.schema.json";
/// Generator recorded in artifact provenance.
pub const RUNTIME_V4_EXPERT_GENERATOR: &str = "hand-authored";

const MANIFEST_BYTES: &[u8] =
    include_bytes!("../../../protocol-artifact/runtime-v4-expert/manifest.json");
const SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../protocol-artifact/runtime-v4-expert/schema.json");
const SOURCE_SCHEMA_BYTES: &[u8] = include_bytes!("../../../schemas/runtime-v4-expert.schema.json");
const CONFORMANCE_BYTES: &[u8] =
    include_bytes!("../../../conformance/cases/runtime-v4-expert.json");
const GOLDEN_BYTES: &[u8] =
    include_bytes!("../../../protocol-artifact/runtime-v4-expert/golden/observation.json");
const CHECKSUMS: &str = include_str!("../../../protocol-artifact/runtime-v4-expert/SHA256SUMS");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeV4ExpertArtifactError {
    ManifestMismatch,
    SchemaMismatch,
    GoldenMismatch,
    ChecksumMismatch,
}

impl std::fmt::Display for RuntimeV4ExpertArtifactError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ManifestMismatch => "runtime-v4-expert manifest mismatch",
            Self::SchemaMismatch => "runtime-v4-expert schema mismatch",
            Self::GoldenMismatch => "runtime-v4-expert golden mismatch",
            Self::ChecksumMismatch => "runtime-v4-expert checksum mismatch",
        })
    }
}

impl std::error::Error for RuntimeV4ExpertArtifactError {}

/// Verifies the copied release-like bytes before an expert-state consumer uses them.
pub fn verify_runtime_v4_expert_artifact() -> Result<(), RuntimeV4ExpertArtifactError> {
    let manifest: Value = serde_json::from_slice(MANIFEST_BYTES)
        .map_err(|_| RuntimeV4ExpertArtifactError::ManifestMismatch)?;
    if !manifest_matches(&manifest) {
        return Err(RuntimeV4ExpertArtifactError::ManifestMismatch);
    }

    let schema: Value = serde_json::from_slice(SCHEMA_BYTES)
        .map_err(|_| RuntimeV4ExpertArtifactError::SchemaMismatch)?;
    if SCHEMA_BYTES != SOURCE_SCHEMA_BYTES || !schema_matches(&schema) {
        return Err(RuntimeV4ExpertArtifactError::SchemaMismatch);
    }

    let golden: Value = serde_json::from_slice(GOLDEN_BYTES)
        .map_err(|_| RuntimeV4ExpertArtifactError::GoldenMismatch)?;
    let conformance: Value = serde_json::from_slice(CONFORMANCE_BYTES)
        .map_err(|_| RuntimeV4ExpertArtifactError::GoldenMismatch)?;
    if !golden_matches(&golden) || !conformance_matches(&conformance) {
        return Err(RuntimeV4ExpertArtifactError::GoldenMismatch);
    }

    if !checksums_match() {
        return Err(RuntimeV4ExpertArtifactError::ChecksumMismatch);
    }
    Ok(())
}

fn manifest_matches(manifest: &Value) -> bool {
    let Some(object) = manifest.as_object() else {
        return false;
    };
    if object.len() != 8
        || ![
            "artifact",
            "protocol_version",
            "schema",
            "schema_digest",
            "provenance",
            "consumers",
            "goldens",
            "checksums",
        ]
        .into_iter()
        .all(|field| object.contains_key(field))
    {
        return false;
    }
    let Some(provenance) = manifest.get("provenance").and_then(Value::as_object) else {
        return false;
    };
    string_field(manifest, "artifact") == Some(RUNTIME_V4_EXPERT_ARTIFACT)
        && string_field(manifest, "protocol_version") == Some(RUNTIME_V4_EXPERT_PROTOCOL_VERSION)
        && string_field(manifest, "schema") == Some("schema.json")
        && string_field(manifest, "schema_digest") == Some(RUNTIME_V4_EXPERT_SCHEMA_DIGEST)
        && provenance.len() == 3
        && provenance.get("source").and_then(Value::as_str) == Some(RUNTIME_V4_EXPERT_SCHEMA_SOURCE)
        && provenance.get("generator").and_then(Value::as_str) == Some(RUNTIME_V4_EXPERT_GENERATOR)
        && provenance.get("license").and_then(Value::as_str) == Some("MIT")
        && manifest
            .get("consumers")
            .and_then(Value::as_array)
            .is_some_and(|consumers| {
                consumers.len() == 4
                    && consumers
                        .iter()
                        .zip([
                            "sts2-game-mod",
                            "sts2-gateway",
                            "sts2-harness",
                            "sts2-mcp-server",
                        ])
                        .all(|(actual, expected)| actual.as_str() == Some(expected))
            })
        && manifest
            .get("goldens")
            .and_then(Value::as_array)
            .is_some_and(|goldens| {
                goldens.len() == 1 && goldens[0].as_str() == Some("golden/observation.json")
            })
        && string_field(manifest, "checksums") == Some("SHA256SUMS")
}

fn schema_matches(schema: &Value) -> bool {
    schema.get("$id").and_then(Value::as_str) == Some("sts2-runtime-v4-expert")
        && schema
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(|properties| {
                properties
                    .get("protocol_version")
                    .and_then(|value| value.get("const"))
                    .and_then(Value::as_str)
                    == Some(RUNTIME_V4_EXPERT_PROTOCOL_VERSION)
            })
        && schema
            .get("$defs")
            .and_then(Value::as_object)
            .is_some_and(|definitions| {
                definitions.contains_key("player")
                    && definitions.contains_key("enemy")
                    && definitions.contains_key("map_node")
                    && definitions.contains_key("action")
            })
}

fn golden_matches(golden: &Value) -> bool {
    golden.get("protocol_version").and_then(Value::as_str)
        == Some(RUNTIME_V4_EXPERT_PROTOCOL_VERSION)
        && golden.get("schema_digest").and_then(Value::as_str)
            == Some(RUNTIME_V4_EXPERT_SCHEMA_DIGEST)
        && golden.get("profile").and_then(Value::as_str) == Some("expert-state")
        && golden
            .get("player")
            .and_then(|player| player.get("block"))
            .is_some()
        && golden
            .get("legal_actions")
            .and_then(Value::as_array)
            .is_some_and(|actions| {
                actions
                    .iter()
                    .any(|action| action["action"]["kind"].as_str() == Some("use_potion"))
            })
}

fn conformance_matches(case: &Value) -> bool {
    case.get("case_id").and_then(Value::as_str) == Some("CT-PROTO-RUNTIME-V4-EXPERT-001")
        && case.get("contract").and_then(Value::as_str) == Some("sts2.protocol/runtime-v4-expert")
        && case["setup"]["live_runtime"] == false
        && case["setup"]["network"] == false
        && case["setup"]["provider"] == false
        && case["setup"]["proprietary_data"] == false
}

fn checksums_match() -> bool {
    let expected = [
        (
            "../../conformance/cases/runtime-v4-expert.json",
            CONFORMANCE_BYTES,
        ),
        (
            "../../schemas/runtime-v4-expert.schema.json",
            SOURCE_SCHEMA_BYTES,
        ),
        ("manifest.json", MANIFEST_BYTES),
        ("schema.json", SCHEMA_BYTES),
        ("golden/observation.json", GOLDEN_BYTES),
    ];
    let mut seen = [false; 5];
    let mut listed = 0;
    for line in CHECKSUMS.lines() {
        let Some((digest, path)) = line.split_once("  ") else {
            return false;
        };
        let Some(index) = expected
            .iter()
            .position(|(expected_path, _)| *expected_path == path)
        else {
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
    listed == expected.len() && seen.into_iter().all(|present| present)
}

fn string_field<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}
