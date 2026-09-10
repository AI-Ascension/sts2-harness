// SPDX-License-Identifier: MIT

use serde_json::Value;
use sha2::{Digest, Sha256};

pub const COOP_RECEIPT_QUERY_PROTOCOL_VERSION: &str = "coop-receipt-query-v1";
pub const COOP_RECEIPT_QUERY_ARTIFACT: &str = "sts2-protocol/coop-receipt-query-v1";
pub const COOP_RECEIPT_QUERY_SCHEMA_SOURCE: &str = "schemas/coop-receipt-query-v1.schema.json";
pub const COOP_RECEIPT_QUERY_SCHEMA_DIGEST: &str =
    "3e3eaedb93926b26025abb09d8028491e2632896753688c1182c698fed7d3f7c";
pub const COOP_RECEIPT_QUERY_GENERATOR: &str = "hand-authored";

const MANIFEST: &[u8] =
    include_bytes!("../../../protocol-artifact/coop-receipt-query-v1/manifest.json");
const SCHEMA: &[u8] =
    include_bytes!("../../../protocol-artifact/coop-receipt-query-v1/schema.json");
const SOURCE_SCHEMA: &[u8] = include_bytes!("../../../schemas/coop-receipt-query-v1.schema.json");
const CONFORMANCE: &[u8] = include_bytes!("../../../conformance/cases/coop-receipt-query-v1.json");
const CHECKSUMS: &str = include_str!("../../../protocol-artifact/coop-receipt-query-v1/SHA256SUMS");
const README: &[u8] = include_bytes!("../../../protocol-artifact/coop-receipt-query-v1/README.md");
const CONFORMANCE_COPY: &[u8] =
    include_bytes!("../../../protocol-artifact/coop-receipt-query-v1/conformance.json");
const MANIFEST_PATH: &str = "manifest.json";
const SCHEMA_PATH: &str = "schema.json";

const FIXTURES: [(&str, &[u8]); 4] = [
    (
        "fixtures/invalid-request-participant-count.json",
        include_bytes!(
            "../../../protocol-artifact/coop-receipt-query-v1/fixtures/invalid-request-participant-count.json"
        ),
    ),
    (
        "fixtures/invalid-request-unknown-member.json",
        include_bytes!(
            "../../../protocol-artifact/coop-receipt-query-v1/fixtures/invalid-request-unknown-member.json"
        ),
    ),
    (
        "fixtures/invalid-response-fresh-scope.json",
        include_bytes!(
            "../../../protocol-artifact/coop-receipt-query-v1/fixtures/invalid-response-fresh-scope.json"
        ),
    ),
    (
        "fixtures/invalid-response-status-receipt.json",
        include_bytes!(
            "../../../protocol-artifact/coop-receipt-query-v1/fixtures/invalid-response-status-receipt.json"
        ),
    ),
];
const GOLDENS: [(&str, &[u8]); 5] = [
    (
        "golden/receipt-query-request.json",
        include_bytes!(
            "../../../protocol-artifact/coop-receipt-query-v1/golden/receipt-query-request.json"
        ),
    ),
    (
        "golden/receipt-query-response-accepted.json",
        include_bytes!(
            "../../../protocol-artifact/coop-receipt-query-v1/golden/receipt-query-response-accepted.json"
        ),
    ),
    (
        "golden/receipt-query-response-rejected.json",
        include_bytes!(
            "../../../protocol-artifact/coop-receipt-query-v1/golden/receipt-query-response-rejected.json"
        ),
    ),
    (
        "golden/receipt-query-response-settled.json",
        include_bytes!(
            "../../../protocol-artifact/coop-receipt-query-v1/golden/receipt-query-response-settled.json"
        ),
    ),
    (
        "golden/receipt-query-response-unknown.json",
        include_bytes!(
            "../../../protocol-artifact/coop-receipt-query-v1/golden/receipt-query-response-unknown.json"
        ),
    ),
];

/// Verifies the exact copied profile before a runtime receipt query is sent.
pub fn verify_coop_receipt_query_artifact() -> Result<(), CoopReceiptQueryArtifactError> {
    if format_digest(SOURCE_SCHEMA) != COOP_RECEIPT_QUERY_SCHEMA_DIGEST || SCHEMA != SOURCE_SCHEMA {
        return Err(CoopReceiptQueryArtifactError::ChecksumMismatch);
    }
    let manifest: Value = serde_json::from_slice(MANIFEST)
        .map_err(|_| CoopReceiptQueryArtifactError::ManifestMismatch)?;
    if manifest["artifact"] != COOP_RECEIPT_QUERY_ARTIFACT
        || manifest["protocol_version"] != COOP_RECEIPT_QUERY_PROTOCOL_VERSION
        || manifest["contract_revision"] != 1
        || manifest["status"] != "proposed_unadmitted"
        || manifest["schema"] != SCHEMA_PATH
        || manifest["schema_digest"] != COOP_RECEIPT_QUERY_SCHEMA_DIGEST
        || manifest["consumers"] != Value::Array(Vec::new())
    {
        return Err(CoopReceiptQueryArtifactError::ManifestMismatch);
    }
    if manifest["provenance"]["source"] != COOP_RECEIPT_QUERY_SCHEMA_SOURCE
        || manifest["provenance"]["generator"] != COOP_RECEIPT_QUERY_GENERATOR
        || manifest["provenance"]["license"] != "MIT"
    {
        return Err(CoopReceiptQueryArtifactError::ManifestMismatch);
    }
    let schema: Value = serde_json::from_slice(SCHEMA)
        .map_err(|_| CoopReceiptQueryArtifactError::SchemaMismatch)?;
    if schema["$id"] != "sts2-coop-receipt-query-v1" {
        return Err(CoopReceiptQueryArtifactError::SchemaMismatch);
    }
    let conformance: Value = serde_json::from_slice(CONFORMANCE)
        .map_err(|_| CoopReceiptQueryArtifactError::ConformanceMismatch)?;
    if conformance["profile"] != COOP_RECEIPT_QUERY_PROTOCOL_VERSION
        || conformance["schema_digest"] != COOP_RECEIPT_QUERY_SCHEMA_DIGEST
        || conformance["status"] != "proposed_unadmitted"
        || conformance["consumers"] != Value::Array(Vec::new())
    {
        return Err(CoopReceiptQueryArtifactError::ConformanceMismatch);
    }
    let mut files: Vec<(&str, &[u8])> = vec![
        (
            "../../conformance/cases/coop-receipt-query-v1.json",
            CONFORMANCE,
        ),
        (
            "../../schemas/coop-receipt-query-v1.schema.json",
            SOURCE_SCHEMA,
        ),
        ("README.md", README),
        ("conformance.json", CONFORMANCE_COPY),
        (MANIFEST_PATH, MANIFEST),
        (SCHEMA_PATH, SCHEMA),
    ];
    files.extend(FIXTURES);
    files.extend(GOLDENS);
    verify_checksums(&files)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoopReceiptQueryArtifactError {
    ManifestMismatch,
    SchemaMismatch,
    ConformanceMismatch,
    ChecksumMismatch,
}

impl std::fmt::Display for CoopReceiptQueryArtifactError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ManifestMismatch => "co-op receipt-query manifest is invalid",
            Self::SchemaMismatch => "co-op receipt-query schema is invalid",
            Self::ConformanceMismatch => "co-op receipt-query conformance case is invalid",
            Self::ChecksumMismatch => "co-op receipt-query checksum is invalid",
        })
    }
}

impl std::error::Error for CoopReceiptQueryArtifactError {}

fn verify_checksums(files: &[(&str, &[u8])]) -> Result<(), CoopReceiptQueryArtifactError> {
    let mut seen = 0;
    for line in CHECKSUMS.lines() {
        let Some((digest, path)) = line.split_once("  ") else {
            return Err(CoopReceiptQueryArtifactError::ChecksumMismatch);
        };
        let Some((_, bytes)) = files.iter().find(|(candidate, _)| *candidate == path) else {
            return Err(CoopReceiptQueryArtifactError::ChecksumMismatch);
        };
        if digest != format_digest(bytes) {
            return Err(CoopReceiptQueryArtifactError::ChecksumMismatch);
        }
        seen += 1;
    }
    if seen != files.len() {
        Err(CoopReceiptQueryArtifactError::ChecksumMismatch)
    } else {
        Ok(())
    }
}

fn format_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::verify_coop_receipt_query_artifact;

    #[test]
    fn copied_neutral_artifact_is_exact() {
        assert_eq!(verify_coop_receipt_query_artifact(), Ok(()));
    }
}
