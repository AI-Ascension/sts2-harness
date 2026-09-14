// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::descriptor::ExoCapabilityDescriptor;
use super::{
    EXO_BRIDGE_WIRE_VERSION, EXO_CAPABILITY_SCHEMA, EXO_CONTRACT_VERSION, EXO_SOURCE_BASE_REVISION,
    EXO_SOURCE_REVISION,
};
use crate::sha256_hex;

const MANIFEST: &str = include_str!("../../../../../protocol-artifact/exo-bridge-v1/manifest.json");
const SCHEMA: &str = include_str!("../../../../../protocol-artifact/exo-bridge-v1/schema.json");
const CONFORMANCE: &str =
    include_str!("../../../../../protocol-artifact/exo-bridge-v1/conformance.json");
const CAPABILITY: &str =
    include_str!("../../../../../protocol-artifact/exo-bridge-v1/golden/capability-source.json");
const README_BYTES: &[u8] =
    include_bytes!("../../../../../protocol-artifact/exo-bridge-v1/README.md");
const MANIFEST_BYTES: &[u8] =
    include_bytes!("../../../../../protocol-artifact/exo-bridge-v1/manifest.json");
const SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../../protocol-artifact/exo-bridge-v1/schema.json");
const CONFORMANCE_BYTES: &[u8] =
    include_bytes!("../../../../../protocol-artifact/exo-bridge-v1/conformance.json");
const CAPABILITY_BYTES: &[u8] =
    include_bytes!("../../../../../protocol-artifact/exo-bridge-v1/golden/capability-source.json");
const DECISION_BYTES: &[u8] =
    include_bytes!("../../../../../protocol-artifact/exo-bridge-v1/golden/decision-action.json");
const REQUEST_BYTES: &[u8] =
    include_bytes!("../../../../../protocol-artifact/exo-bridge-v1/golden/request.json");
const INVALID_BYTES: &[u8] =
    include_bytes!("../../../../../protocol-artifact/exo-bridge-v1/fixtures/invalid-unknown.json");

/// Returns the checked-in immutable source/package manifest.
#[must_use]
pub const fn exo_bridge_manifest() -> &'static str {
    MANIFEST
}

/// Verifies the source-derived manifest, descriptor, schema digest, and fixture checksums.
pub fn verify_exo_bridge_artifact() -> Result<(), ExoArtifactError> {
    let manifest: Value = serde_json::from_str(MANIFEST).map_err(|_| ExoArtifactError::Manifest)?;
    if manifest.get("artifact").and_then(Value::as_str) != Some("sts2-harness/exo-bridge-v1")
        || manifest.get("manifest_schema").and_then(Value::as_str) != Some("sts2.exo-manifest-v1")
        || manifest.get("contract_version").and_then(Value::as_str) != Some(EXO_CONTRACT_VERSION)
        || manifest.get("capability_schema").and_then(Value::as_str) != Some(EXO_CAPABILITY_SCHEMA)
        || manifest.get("schema").and_then(Value::as_str) != Some("schema.json")
        || manifest.get("schema_sha256").and_then(Value::as_str)
            != Some(sha256_hex(SCHEMA_BYTES).as_str())
    {
        return Err(ExoArtifactError::Manifest);
    }
    let source = manifest
        .get("source")
        .and_then(Value::as_object)
        .ok_or(ExoArtifactError::Manifest)?;
    if source.get("base_revision").and_then(Value::as_str) != Some(EXO_SOURCE_BASE_REVISION)
        || source.get("candidate_revision").and_then(Value::as_str) != Some(EXO_SOURCE_REVISION)
        || source.get("candidate_tree").and_then(Value::as_str)
            != Some("f1c155c8b9b1c2ee83a34a04189e64203536b3ab")
        || source
            .get("reviewed_commits")
            .and_then(Value::as_array)
            .is_none_or(|commits| commits.len() != 9)
    {
        return Err(ExoArtifactError::SourceReview);
    }
    let descriptor: ExoCapabilityDescriptor =
        serde_json::from_str(CAPABILITY).map_err(|_| ExoArtifactError::Descriptor)?;
    descriptor
        .validate()
        .map_err(|_| ExoArtifactError::Descriptor)?;
    let schema: Value = serde_json::from_str(SCHEMA).map_err(|_| ExoArtifactError::Schema)?;
    if schema.get("$id").and_then(Value::as_str) != Some("sts2-harness/exo-bridge/v1") {
        return Err(ExoArtifactError::Schema);
    }
    let defs = schema
        .get("$defs")
        .and_then(Value::as_object)
        .ok_or(ExoArtifactError::Schema)?;
    if [
        "request_envelope",
        "decision_envelope",
        "decision_request",
        "decision",
    ]
    .iter()
    .any(|name| !defs.contains_key(*name))
    {
        return Err(ExoArtifactError::Schema);
    }
    let conformance: Value =
        serde_json::from_str(CONFORMANCE).map_err(|_| ExoArtifactError::Fixture)?;
    if conformance.get("wire_version").and_then(Value::as_str) != Some(EXO_BRIDGE_WIRE_VERSION) {
        return Err(ExoArtifactError::Fixture);
    }
    if [
        "request_vectors",
        "decision_vectors",
        "envelope_vectors",
        "capability_vectors",
    ]
    .iter()
    .any(|name| {
        conformance
            .get(*name)
            .and_then(Value::as_array)
            .is_none_or(|vectors| vectors.is_empty())
    }) {
        return Err(ExoArtifactError::Fixture);
    }
    let checksums = [
        ("README.md", README_BYTES),
        ("manifest.json", MANIFEST_BYTES),
        ("schema.json", SCHEMA_BYTES),
        ("conformance.json", CONFORMANCE_BYTES),
        ("golden/capability-source.json", CAPABILITY_BYTES),
        ("golden/decision-action.json", DECISION_BYTES),
        ("golden/request.json", REQUEST_BYTES),
        ("fixtures/invalid-unknown.json", INVALID_BYTES),
    ];
    let sums = [
        (
            checksums[0].0,
            "2016b2bea7d04f9f4727e5faa35854b5bbf679d2f3e402a69d62c4f9b8e8cbef",
        ),
        (
            checksums[1].0,
            "a2e41321b9563d8c960766f5d58855c2392c67606e6ec854d163b84e741b59e8",
        ),
        (
            checksums[2].0,
            "046336e4c436ea0a073f2f8ee46e7b5f214f479af911d378c0839431edcf271a",
        ),
        (
            checksums[3].0,
            "dc30cbc19d72b6a8627c1cb3cc9686cd60db5ab6044f7c50bd2e45cebbbc526f",
        ),
        (
            checksums[4].0,
            "ec0c05534de7fb9c3757dbb22fc987c0a5786f3319a51b28f3b0d6c17249979e",
        ),
        (
            checksums[5].0,
            "88dfd6387f8596c1c01535480969328e150ba00215b32d391be63d244a946652",
        ),
        (
            checksums[6].0,
            "38665742d1bd85f9c9377ce11f7365d47b67cf51c16333e44013267a2496786f",
        ),
        (
            checksums[7].0,
            "0cf73e8e314251ead61357a5534e0442ebd551e2b63feffcdde89d8e470abf17",
        ),
    ];
    if checksums
        .into_iter()
        .zip(sums)
        .any(|((path, bytes), (sum_path, expected))| {
            path != sum_path || sha256_hex(bytes) != expected
        })
    {
        return Err(ExoArtifactError::Checksum);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExoArtifactError {
    Manifest,
    SourceReview,
    Schema,
    Descriptor,
    Fixture,
    Checksum,
}

impl std::fmt::Display for ExoArtifactError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Manifest => "Exo bridge manifest is invalid",
            Self::SourceReview => "Exo bridge source review is incomplete",
            Self::Schema => "Exo bridge schema is invalid",
            Self::Descriptor => "Exo bridge capability fixture is invalid",
            Self::Fixture => "Exo bridge conformance fixture is invalid",
            Self::Checksum => "Exo bridge fixture checksum is invalid",
        })
    }
}

impl std::error::Error for ExoArtifactError {}
