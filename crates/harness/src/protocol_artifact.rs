// SPDX-License-Identifier: MIT

use serde_json::Value;
use sha2::{Digest, Sha256};

/// Version consumed by the deterministic harness POC.
pub const POC_PROTOCOL_VERSION: &str = "poc-v1";
/// Schema digest supplied by the protocol release-like artifact.
pub const POC_SCHEMA_DIGEST: &str =
    "242b8f9233e915a55ea8d2e72ca476c1258169a67e62de72ee5aed848a6a0a19";
/// Release-like artifact identity, not a Rust package dependency.
pub const POC_ARTIFACT: &str = "sts2-protocol/poc-v1";
/// Repository-relative source recorded in the artifact provenance.
pub const POC_SCHEMA_SOURCE: &str = "schemas/poc-v1.schema.json";
/// Generator recorded in the hand-authored artifact.
pub const POC_GENERATOR: &str = "hand-authored";
/// Maximum fake budget represented by the bounded contract.
pub const POC_MAX_UNITS: u16 = 8;
/// Maximum settled-effect count represented by the bounded contract.
pub const POC_MAX_SETTLED_EFFECTS: u16 = 4;
/// Maximum JSON-safe generation represented by the bounded contract.
pub const POC_MAX_GENERATION: u64 = 9_007_199_254_740_991;

const MANIFEST: &str = include_str!("../../../protocol-artifact/poc-v1/manifest.json");
const SCHEMA: &str = include_str!("../../../protocol-artifact/poc-v1/schema.json");
const STATE_REQUEST: &str =
    include_str!("../../../protocol-artifact/poc-v1/golden/state-request.json");
const STATE_RESPONSE: &str =
    include_str!("../../../protocol-artifact/poc-v1/golden/state-response.json");
const ACTION_REQUEST: &str =
    include_str!("../../../protocol-artifact/poc-v1/golden/action-request.json");
const ACTION_RESPONSE: &str =
    include_str!("../../../protocol-artifact/poc-v1/golden/action-accepted.json");
const ACTION_REJECTED: &str =
    include_str!("../../../protocol-artifact/poc-v1/golden/action-rejected.json");
const INVALID_ACTION: &str =
    include_str!("../../../protocol-artifact/poc-v1/fixtures/invalid-action.json");
const CONFORMANCE: &str = include_str!("../../../conformance/cases/poc-v1.json");
const CHECKSUMS: &str = include_str!("../../../protocol-artifact/poc-v1/SHA256SUMS");

pub(crate) const POC_STATE_RESPONSE: &str = STATE_RESPONSE;
pub(crate) const POC_ACTION_RESPONSE: &str = ACTION_RESPONSE;
pub(crate) const POC_INVALID_ACTION: &str = INVALID_ACTION;

const MANIFEST_BYTES: &[u8] = include_bytes!("../../../protocol-artifact/poc-v1/manifest.json");
const SCHEMA_BYTES: &[u8] = include_bytes!("../../../protocol-artifact/poc-v1/schema.json");
const SOURCE_SCHEMA_BYTES: &[u8] = include_bytes!("../../../schemas/poc-v1.schema.json");
const CONFORMANCE_BYTES: &[u8] = include_bytes!("../../../conformance/cases/poc-v1.json");
const STATE_REQUEST_BYTES: &[u8] =
    include_bytes!("../../../protocol-artifact/poc-v1/golden/state-request.json");
const STATE_RESPONSE_BYTES: &[u8] =
    include_bytes!("../../../protocol-artifact/poc-v1/golden/state-response.json");
const ACTION_REQUEST_BYTES: &[u8] =
    include_bytes!("../../../protocol-artifact/poc-v1/golden/action-request.json");
const ACTION_RESPONSE_BYTES: &[u8] =
    include_bytes!("../../../protocol-artifact/poc-v1/golden/action-accepted.json");
const ACTION_REJECTED_BYTES: &[u8] =
    include_bytes!("../../../protocol-artifact/poc-v1/golden/action-rejected.json");
const INVALID_ACTION_BYTES: &[u8] =
    include_bytes!("../../../protocol-artifact/poc-v1/fixtures/invalid-action.json");

const EXPECTED_CONSUMERS: [&str; 5] = [
    "sts2-game-core",
    "sts2-game-mod",
    "sts2-gateway",
    "sts2-harness",
    "sts2-mcp-server",
];

/// Verifies the local copied artifact before the deterministic runner uses it.
pub fn verify_poc_artifact() -> Result<(), ArtifactError> {
    let manifest: Value =
        serde_json::from_str(MANIFEST).map_err(|_| ArtifactError::ManifestMismatch)?;
    if !manifest_matches(&manifest) {
        return Err(ArtifactError::ManifestMismatch);
    }

    let schema: Value = serde_json::from_str(SCHEMA).map_err(|_| ArtifactError::SchemaMismatch)?;
    if !schema_matches(&schema) {
        return Err(ArtifactError::SchemaMismatch);
    }

    let fixtures = [
        (STATE_REQUEST, "state_request"),
        (STATE_RESPONSE, "state_response"),
        (ACTION_REQUEST, "action_request"),
        (ACTION_RESPONSE, "action_response"),
        (ACTION_REJECTED, "action_response"),
        (INVALID_ACTION, "action_request"),
    ];
    for (fixture, expected_kind) in fixtures {
        let fixture: Value =
            serde_json::from_str(fixture).map_err(|_| ArtifactError::FixtureMismatch)?;
        if !fixture_matches(&fixture, expected_kind) {
            return Err(ArtifactError::FixtureMismatch);
        }
    }
    let conformance: Value =
        serde_json::from_str(CONFORMANCE).map_err(|_| ArtifactError::FixtureMismatch)?;
    if !conformance_matches(&conformance) {
        return Err(ArtifactError::FixtureMismatch);
    }

    let checksums = [
        ("../../conformance/cases/poc-v1.json", CONFORMANCE_BYTES),
        ("../../schemas/poc-v1.schema.json", SOURCE_SCHEMA_BYTES),
        ("fixtures/invalid-action.json", INVALID_ACTION_BYTES),
        ("golden/action-accepted.json", ACTION_RESPONSE_BYTES),
        ("golden/action-rejected.json", ACTION_REJECTED_BYTES),
        ("golden/action-request.json", ACTION_REQUEST_BYTES),
        ("golden/state-request.json", STATE_REQUEST_BYTES),
        ("golden/state-response.json", STATE_RESPONSE_BYTES),
        ("manifest.json", MANIFEST_BYTES),
        ("schema.json", SCHEMA_BYTES),
    ];
    if checksums
        .into_iter()
        .any(|(path, bytes)| !checksum_matches(path, bytes))
    {
        return Err(ArtifactError::ChecksumMismatch);
    }

    Ok(())
}

fn manifest_matches(manifest: &Value) -> bool {
    let Some(object) = manifest.as_object() else {
        return false;
    };
    if object.len() != 6
        || ![
            "artifact",
            "protocol_version",
            "schema",
            "schema_digest",
            "provenance",
            "consumers",
        ]
        .into_iter()
        .all(|key| object.contains_key(key))
    {
        return false;
    }

    if string_field(manifest, "artifact") != Some(POC_ARTIFACT)
        || string_field(manifest, "protocol_version") != Some(POC_PROTOCOL_VERSION)
        || string_field(manifest, "schema") != Some("schema.json")
        || string_field(manifest, "schema_digest") != Some(POC_SCHEMA_DIGEST)
    {
        return false;
    }

    let Some(provenance) = manifest.get("provenance").and_then(Value::as_object) else {
        return false;
    };
    if provenance.len() != 3
        || provenance.get("source").and_then(Value::as_str) != Some(POC_SCHEMA_SOURCE)
        || provenance.get("generator").and_then(Value::as_str) != Some(POC_GENERATOR)
        || provenance.get("license").and_then(Value::as_str) != Some("MIT")
    {
        return false;
    }

    manifest
        .get("consumers")
        .and_then(Value::as_array)
        .is_some_and(|consumers| {
            consumers.len() == EXPECTED_CONSUMERS.len()
                && consumers
                    .iter()
                    .zip(EXPECTED_CONSUMERS)
                    .all(|(actual, expected)| actual.as_str() == Some(expected))
        })
}

fn schema_matches(schema: &Value) -> bool {
    let Some(definitions) = schema.get("$defs").and_then(Value::as_object) else {
        return false;
    };
    let Some(provenance) = definitions.get("provenance").and_then(Value::as_object) else {
        return false;
    };
    let Some(properties) = provenance.get("properties").and_then(Value::as_object) else {
        return false;
    };
    let Some(base) = definitions.get("base").and_then(Value::as_object) else {
        return false;
    };
    let Some(base_properties) = base.get("properties").and_then(Value::as_object) else {
        return false;
    };

    schema.get("$id").and_then(Value::as_str) == Some("sts2-poc-v1")
        && schema
            .get("oneOf")
            .and_then(Value::as_array)
            .is_some_and(|messages| messages.len() == 4)
        && properties
            .get("artifact")
            .and_then(|value| value.get("const"))
            .and_then(Value::as_str)
            == Some(POC_ARTIFACT)
        && properties
            .get("source")
            .and_then(|value| value.get("const"))
            .and_then(Value::as_str)
            == Some(POC_SCHEMA_SOURCE)
        && properties
            .get("generator")
            .and_then(|value| value.get("const"))
            .and_then(Value::as_str)
            == Some(POC_GENERATOR)
        && base_properties
            .get("generation")
            .and_then(|value| value.get("maximum"))
            .and_then(Value::as_u64)
            == Some(POC_MAX_GENERATION)
}

fn fixture_matches(fixture: &Value, expected_kind: &str) -> bool {
    string_field(fixture, "protocol_version") == Some(POC_PROTOCOL_VERSION)
        && string_field(fixture, "schema_digest") == Some(POC_SCHEMA_DIGEST)
        && string_field(fixture, "kind") == Some(expected_kind)
        && fixture
            .get("provenance")
            .and_then(Value::as_object)
            .is_some_and(|provenance| {
                provenance.get("artifact").and_then(Value::as_str) == Some(POC_ARTIFACT)
                    && provenance.get("source").and_then(Value::as_str) == Some(POC_SCHEMA_SOURCE)
                    && provenance.get("generator").and_then(Value::as_str) == Some(POC_GENERATOR)
            })
}

fn conformance_matches(conformance: &Value) -> bool {
    string_field(conformance, "case_id") == Some("CT-POC-V1-001")
        && string_field(conformance, "contract") == Some("sts2.protocol/poc-v1")
        && string_field(conformance, "profile") == Some(POC_PROTOCOL_VERSION)
        && string_field(conformance, "schema") == Some(POC_SCHEMA_SOURCE)
        && string_field(conformance, "invalid")
            == Some("artifacts/poc-v1/fixtures/invalid-action.json")
        && conformance
            .get("goldens")
            .and_then(Value::as_array)
            .is_some_and(|goldens| goldens.len() == 5)
        && string_field(conformance, "checksums") == Some("artifacts/poc-v1/SHA256SUMS")
}

fn string_field<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
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

/// A deterministic failure while loading the copied artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactError {
    ManifestMismatch,
    SchemaMismatch,
    FixtureMismatch,
    ChecksumMismatch,
}

impl std::fmt::Display for ArtifactError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("copied POC artifact is invalid")
    }
}

impl std::error::Error for ArtifactError {}
