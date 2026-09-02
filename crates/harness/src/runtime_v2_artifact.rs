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

include!("runtime_v2_artifact_verify.rs");
