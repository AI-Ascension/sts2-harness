// SPDX-License-Identifier: MIT

use serde_json::Value;
use sha2::{Digest as Sha2Digest, Sha256};

/// The Runtime-v2 profile consumed by the deterministic fake lane.
pub const RUNTIME_V2_PROTOCOL_VERSION: &str = "runtime-v2";
/// The release-like artifact identity. This is not a Rust package dependency.
pub const RUNTIME_V2_ARTIFACT: &str = "sts2-protocol/runtime-v2";
/// The source identity recorded in the copied artifact provenance.
pub const RUNTIME_V2_SCHEMA_SOURCE: &str = "schemas/runtime-v2.schema.json";
/// The generator identity recorded in the copied artifact provenance.
pub const RUNTIME_V2_GENERATOR: &str = "hand-authored";
/// The byte-exact schema digest handed off by `sts2-protocol` commit
/// `8d4b2f574cf860a71f2a5e4ce3308ac069cb1527`. Replace this module as one unit for a later release.
pub const RUNTIME_V2_SCHEMA_DIGEST: &str =
    "f7963b19c8ed5bbdc02c08e83c7a2e16c4771ed5eb798b29a8208d7a917a86c2";

/// Maximum fake turn index permitted by the bounded Runtime-v2 contract.
pub const RUNTIME_V2_MAX_TURN_INDEX: u64 = 1024;
/// Maximum fake generation permitted by the bounded Runtime-v2 contract.
pub const RUNTIME_V2_MAX_GENERATION: u64 = 9_007_199_254_740_991;
/// Maximum fake lease epoch permitted by the bounded Runtime-v2 contract.
pub const RUNTIME_V2_MAX_LEASE_EPOCH: u64 = 9_007_199_254_740_991;
/// Maximum identity length permitted by the copied contract.
pub const RUNTIME_V2_MAX_IDENTITY_BYTES: usize = 128;
/// Maximum number of trajectory records emitted by the fake.
pub const RUNTIME_V2_MAX_RECORDS: usize = 32;

const MANIFEST_BYTES: &[u8] = include_bytes!("../../../protocol-artifact/runtime-v2/manifest.json");
const SCHEMA_BYTES: &[u8] = include_bytes!("../../../protocol-artifact/runtime-v2/schema.json");
const SOURCE_SCHEMA_BYTES: &[u8] = include_bytes!("../../../schemas/runtime-v2.schema.json");
const CONFORMANCE_BYTES: &[u8] = include_bytes!("../../../conformance/cases/runtime-v2.json");
const CHECKSUMS: &str = include_str!("../../../protocol-artifact/runtime-v2/SHA256SUMS");
const GOLDEN_BYTES: [&[u8]; 19] = [
    include_bytes!("../../../protocol-artifact/runtime-v2/golden/cancelled-before-dispatch.json"),
    include_bytes!("../../../protocol-artifact/runtime-v2/golden/duplicate-replay.json"),
    include_bytes!("../../../protocol-artifact/runtime-v2/golden/enemy-turn-request.json"),
    include_bytes!("../../../protocol-artifact/runtime-v2/golden/enemy-turn-response.json"),
    include_bytes!(
        "../../../protocol-artifact/runtime-v2/golden/idempotency-conflict-request.json"
    ),
    include_bytes!(
        "../../../protocol-artifact/runtime-v2/golden/idempotency-conflict-response.json"
    ),
    include_bytes!("../../../protocol-artifact/runtime-v2/golden/legal-action-accepted.json"),
    include_bytes!("../../../protocol-artifact/runtime-v2/golden/legal-action-request.json"),
    include_bytes!("../../../protocol-artifact/runtime-v2/golden/legal-action-settled.json"),
    include_bytes!("../../../protocol-artifact/runtime-v2/golden/outside-combat-request.json"),
    include_bytes!("../../../protocol-artifact/runtime-v2/golden/outside-combat-response.json"),
    include_bytes!("../../../protocol-artifact/runtime-v2/golden/reconcile-request.json"),
    include_bytes!("../../../protocol-artifact/runtime-v2/golden/reconcile-settled-response.json"),
    include_bytes!("../../../protocol-artifact/runtime-v2/golden/stale-generation-request.json"),
    include_bytes!("../../../protocol-artifact/runtime-v2/golden/stale-generation-response.json"),
    include_bytes!("../../../protocol-artifact/runtime-v2/golden/state-request.json"),
    include_bytes!("../../../protocol-artifact/runtime-v2/golden/state-response.json"),
    include_bytes!("../../../protocol-artifact/runtime-v2/golden/timeout-action-request.json"),
    include_bytes!("../../../protocol-artifact/runtime-v2/golden/timeout-unknown-response.json"),
];

const GOLDEN_PATHS: [&str; 19] = [
    "golden/cancelled-before-dispatch.json",
    "golden/duplicate-replay.json",
    "golden/enemy-turn-request.json",
    "golden/enemy-turn-response.json",
    "golden/idempotency-conflict-request.json",
    "golden/idempotency-conflict-response.json",
    "golden/legal-action-accepted.json",
    "golden/legal-action-request.json",
    "golden/legal-action-settled.json",
    "golden/outside-combat-request.json",
    "golden/outside-combat-response.json",
    "golden/reconcile-request.json",
    "golden/reconcile-settled-response.json",
    "golden/stale-generation-request.json",
    "golden/stale-generation-response.json",
    "golden/state-request.json",
    "golden/state-response.json",
    "golden/timeout-action-request.json",
    "golden/timeout-unknown-response.json",
];

const MANIFEST_GOLDEN_PATHS: [&str; 19] = [
    "golden/state-request.json",
    "golden/state-response.json",
    "golden/legal-action-request.json",
    "golden/legal-action-accepted.json",
    "golden/legal-action-settled.json",
    "golden/stale-generation-request.json",
    "golden/stale-generation-response.json",
    "golden/outside-combat-request.json",
    "golden/outside-combat-response.json",
    "golden/enemy-turn-request.json",
    "golden/enemy-turn-response.json",
    "golden/idempotency-conflict-request.json",
    "golden/idempotency-conflict-response.json",
    "golden/cancelled-before-dispatch.json",
    "golden/timeout-action-request.json",
    "golden/timeout-unknown-response.json",
    "golden/reconcile-request.json",
    "golden/reconcile-settled-response.json",
    "golden/duplicate-replay.json",
];

#[must_use]
pub fn runtime_v2_schema_bytes() -> &'static [u8] {
    SCHEMA_BYTES
}

#[must_use]
pub fn runtime_v2_manifest_bytes() -> &'static [u8] {
    MANIFEST_BYTES
}

/// A deterministic failure while loading the copied Runtime-v2 artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeV2ArtifactError {
    ManifestMismatch,
    SchemaMismatch,
    ChecksumMismatch,
}

impl std::fmt::Display for RuntimeV2ArtifactError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::ManifestMismatch => "copied Runtime-v2 manifest is invalid",
            Self::SchemaMismatch => "copied Runtime-v2 schema is invalid",
            Self::ChecksumMismatch => "copied Runtime-v2 checksum is invalid",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for RuntimeV2ArtifactError {}

/// Verifies the copied release-like bytes before the fake runner uses them.
pub fn verify_runtime_v2_artifact() -> Result<(), RuntimeV2ArtifactError> {
    let manifest: Value = serde_json::from_slice(MANIFEST_BYTES)
        .map_err(|_| RuntimeV2ArtifactError::ManifestMismatch)?;
    if !manifest_matches(&manifest) {
        return Err(RuntimeV2ArtifactError::ManifestMismatch);
    }

    let schema: Value =
        serde_json::from_slice(SCHEMA_BYTES).map_err(|_| RuntimeV2ArtifactError::SchemaMismatch)?;
    if !schema_matches(&schema) || SCHEMA_BYTES != SOURCE_SCHEMA_BYTES {
        return Err(RuntimeV2ArtifactError::SchemaMismatch);
    }

    if !checksum_matches("../../conformance/cases/runtime-v2.json", CONFORMANCE_BYTES)
        || !checksum_matches("../../schemas/runtime-v2.schema.json", SOURCE_SCHEMA_BYTES)
        || !checksum_matches("manifest.json", MANIFEST_BYTES)
        || !checksum_matches("schema.json", SCHEMA_BYTES)
        || GOLDEN_PATHS
            .into_iter()
            .zip(GOLDEN_BYTES)
            .any(|(path, bytes)| !checksum_matches(path, bytes))
    {
        return Err(RuntimeV2ArtifactError::ChecksumMismatch);
    }

    let schema_digest = Sha256::digest(SCHEMA_BYTES);
    if format!("{schema_digest:x}") != RUNTIME_V2_SCHEMA_DIGEST {
        return Err(RuntimeV2ArtifactError::ChecksumMismatch);
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
        .all(|key| object.contains_key(key))
    {
        return false;
    }

    if string_field(manifest, "artifact") != Some(RUNTIME_V2_ARTIFACT)
        || string_field(manifest, "protocol_version") != Some(RUNTIME_V2_PROTOCOL_VERSION)
        || string_field(manifest, "schema") != Some("schema.json")
        || string_field(manifest, "schema_digest") != Some(RUNTIME_V2_SCHEMA_DIGEST)
    {
        return false;
    }

    let Some(provenance) = manifest.get("provenance").and_then(Value::as_object) else {
        return false;
    };
    if provenance.len() != 3
        || provenance.get("source").and_then(Value::as_str) != Some(RUNTIME_V2_SCHEMA_SOURCE)
        || provenance.get("generator").and_then(Value::as_str) != Some(RUNTIME_V2_GENERATOR)
        || provenance.get("license").and_then(Value::as_str) != Some("MIT")
    {
        return false;
    }

    let consumers_match = manifest
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
        });
    let goldens_match = manifest
        .get("goldens")
        .and_then(Value::as_array)
        .is_some_and(|goldens| {
            goldens.len() == GOLDEN_PATHS.len()
                && goldens
                    .iter()
                    .zip(MANIFEST_GOLDEN_PATHS)
                    .all(|(actual, expected)| actual.as_str() == Some(expected))
        });
    consumers_match && goldens_match && string_field(manifest, "checksums") == Some("SHA256SUMS")
}

fn schema_matches(schema: &Value) -> bool {
    let Some(definitions) = schema.get("$defs").and_then(Value::as_object) else {
        return false;
    };
    let Some(base) = definitions.get("base").and_then(Value::as_object) else {
        return false;
    };
    let Some(base_required) = base.get("required").and_then(Value::as_array) else {
        return false;
    };
    let Some(action) = definitions.get("action").and_then(Value::as_object) else {
        return false;
    };
    let Some(action_properties) = action.get("properties").and_then(Value::as_object) else {
        return false;
    };
    let Some(observation) = definitions.get("observation").and_then(Value::as_object) else {
        return false;
    };
    let Some(observation_properties) = observation.get("properties").and_then(Value::as_object)
    else {
        return false;
    };
    let Some(kinds) = schema.get("oneOf").and_then(Value::as_array) else {
        return false;
    };

    schema.get("$id").and_then(Value::as_str) == Some("sts2-runtime-v2")
        && kinds.len() == 6
        && [
            "protocol_version",
            "schema_digest",
            "provenance",
            "correlation_id",
            "instance_id",
            "session_id",
            "lease_id",
            "lease_epoch",
            "generation",
        ]
        .into_iter()
        .all(|field| {
            base_required
                .iter()
                .any(|value| value.as_str() == Some(field))
        })
        && action_properties
            .get("action_id")
            .and_then(|value| value.get("const"))
            .and_then(Value::as_str)
            == Some("end_turn")
        && observation_properties
            .get("turn_index")
            .and_then(|value| value.get("maximum"))
            .and_then(Value::as_u64)
            == Some(RUNTIME_V2_MAX_TURN_INDEX)
        && observation_properties
            .get("generation")
            .and_then(|value| value.get("maximum"))
            .and_then(Value::as_u64)
            == Some(RUNTIME_V2_MAX_GENERATION)
}

fn checksum_matches(path: &str, bytes: &[u8]) -> bool {
    let Some(expected) = checksum_for(path) else {
        return false;
    };
    let actual = Sha256::digest(bytes);
    format!("{actual:x}") == expected
}

fn checksum_for(path: &str) -> Option<&str> {
    let mut found = None;
    for line in CHECKSUMS.lines() {
        let (digest, listed_path) = line.split_once("  ")?;
        if listed_path == path {
            if found.is_some() {
                return None;
            }
            found = Some(digest);
        }
    }
    found.filter(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn string_field<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}
