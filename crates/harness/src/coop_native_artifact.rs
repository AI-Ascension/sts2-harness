// SPDX-License-Identifier: MIT

use super::identity::CoopNativeLineage;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

/// Version of the accepted source and serialization component contract.
pub const COOP_NATIVE_PROTOCOL_VERSION: &str = "coop-native-v1";
pub const COOP_NATIVE_ARTIFACT: &str = "sts2-protocol/coop-native-v1";
pub const COOP_NATIVE_SCHEMA_SOURCE: &str = "schemas/coop-native-v1.schema.json";
pub const COOP_NATIVE_GENERATOR: &str = "hand-authored";
pub const COOP_NATIVE_SCHEMA_DIGEST: &str =
    "2f3bc99e53080fa11b39592b64fb0ab964a16f568719a2622d0b2caf766ab629";
pub const COOP_NATIVE_PRODUCER_SCHEMA_DIGEST: &str = COOP_NATIVE_SCHEMA_DIGEST;
pub const COOP_NATIVE_MAX_REQUEST_BYTES: usize = 64 * 1024;
pub const COOP_NATIVE_MAX_RESPONSE_BYTES: usize = 128 * 1024;
pub const COOP_NATIVE_MAX_RECORDS: usize = 256;

const MANIFEST: &[u8] = include_bytes!("../../../protocol-artifact/coop-native-v1/manifest.json");
const SCHEMA: &[u8] = include_bytes!("../../../protocol-artifact/coop-native-v1/schema.json");
const SOURCE_SCHEMA: &[u8] = include_bytes!("../../../schemas/coop-native-v1.schema.json");
const CONFORMANCE: &[u8] = include_bytes!("../../../conformance/cases/coop-native-v1.json");
const CONFORMANCE_COPY: &[u8] =
    include_bytes!("../../../protocol-artifact/coop-native-v1/conformance.json");
const CONSUMER_CONFORMANCE: &[u8] =
    include_bytes!("../../../protocol-artifact/coop-native-v1/consumer-conformance.json");
const PRODUCER_CAPTURE: &[u8] =
    include_bytes!("../../../protocol-artifact/coop-native-v1/producer-capture.json");
const README: &[u8] = include_bytes!("../../../protocol-artifact/coop-native-v1/README.md");
const CHECKSUMS: &str =
    include_str!("../../../protocol-artifact/coop-native-v1/SHA256SUMS");

const GOLDENS: [(&str, &[u8]); 17] = [
    (
        "golden/observation-response.json",
        include_bytes!("../../../protocol-artifact/coop-native-v1/golden/observation-response.json"),
    ),
    (
        "golden/legal-catalog-request.json",
        include_bytes!("../../../protocol-artifact/coop-native-v1/golden/legal-catalog-request.json"),
    ),
    (
        "golden/legal-catalog-response.json",
        include_bytes!("../../../protocol-artifact/coop-native-v1/golden/legal-catalog-response.json"),
    ),
    (
        "golden/local-action-settled-request.json",
        include_bytes!("../../../protocol-artifact/coop-native-v1/golden/local-action-settled-request.json"),
    ),
    (
        "golden/local-action-settled-response.json",
        include_bytes!("../../../protocol-artifact/coop-native-v1/golden/local-action-settled-response.json"),
    ),
    (
        "golden/local-action-rejected-request.json",
        include_bytes!("../../../protocol-artifact/coop-native-v1/golden/local-action-rejected-request.json"),
    ),
    (
        "golden/local-action-rejected-response.json",
        include_bytes!("../../../protocol-artifact/coop-native-v1/golden/local-action-rejected-response.json"),
    ),
    (
        "golden/local-action-unknown-request.json",
        include_bytes!("../../../protocol-artifact/coop-native-v1/golden/local-action-unknown-request.json"),
    ),
    (
        "golden/local-action-unknown-response.json",
        include_bytes!("../../../protocol-artifact/coop-native-v1/golden/local-action-unknown-response.json"),
    ),
    (
        "golden/local-action-recovered-request.json",
        include_bytes!("../../../protocol-artifact/coop-native-v1/golden/local-action-recovered-request.json"),
    ),
    (
        "golden/local-action-recovered-response.json",
        include_bytes!("../../../protocol-artifact/coop-native-v1/golden/local-action-recovered-response.json"),
    ),
    (
        "golden/shared-vote-settled-request.json",
        include_bytes!("../../../protocol-artifact/coop-native-v1/golden/shared-vote-settled-request.json"),
    ),
    (
        "golden/shared-vote-settled-response.json",
        include_bytes!("../../../protocol-artifact/coop-native-v1/golden/shared-vote-settled-response.json"),
    ),
    (
        "golden/rejoin-pending-request.json",
        include_bytes!("../../../protocol-artifact/coop-native-v1/golden/rejoin-pending-request.json"),
    ),
    (
        "golden/rejoin-pending-response.json",
        include_bytes!("../../../protocol-artifact/coop-native-v1/golden/rejoin-pending-response.json"),
    ),
    (
        "golden/rejoin-recovered-request.json",
        include_bytes!("../../../protocol-artifact/coop-native-v1/golden/rejoin-recovered-request.json"),
    ),
    (
        "golden/rejoin-recovered-response.json",
        include_bytes!("../../../protocol-artifact/coop-native-v1/golden/rejoin-recovered-response.json"),
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum CoopNativeArtifactStatus {
    #[serde(rename = "accepted_component")]
    AcceptedComponent,
    /// Retained for source compatibility with the earlier candidate adapter.
    #[serde(rename = "candidate")]
    Candidate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum CoopNativeAdmissionStatus {
    #[serde(rename = "component")]
    Component,
    /// Retained for source compatibility with the earlier candidate adapter.
    #[serde(rename = "unadmitted")]
    Unadmitted,
    #[serde(rename = "admitted")]
    Admitted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoopNativeArtifactState {
    status: CoopNativeArtifactStatus,
    admission: CoopNativeAdmissionStatus,
    producer_digest_matches_candidate: bool,
}

impl CoopNativeArtifactState {
    #[must_use]
    pub const fn status(&self) -> CoopNativeArtifactStatus {
        self.status
    }

    #[must_use]
    pub const fn admission(&self) -> CoopNativeAdmissionStatus {
        self.admission
    }

    #[must_use]
    pub const fn is_admitted(&self) -> bool {
        matches!(
            self.admission,
            CoopNativeAdmissionStatus::Component | CoopNativeAdmissionStatus::Admitted
        )
    }

    #[must_use]
    pub const fn producer_digest_matches_candidate(&self) -> bool {
        self.producer_digest_matches_candidate
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoopNativeArtifactError {
    ManifestMismatch,
    SchemaMismatch,
    ConformanceMismatch,
    GoldenMismatch,
    ChecksumMismatch,
    ProducerDigestMismatch,
    Unadmitted,
    ArtifactTooLarge,
}

impl std::fmt::Display for CoopNativeArtifactError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ManifestMismatch => "native co-op component manifest is invalid",
            Self::SchemaMismatch => "native co-op component schema is invalid",
            Self::ConformanceMismatch => "native co-op component conformance is invalid",
            Self::GoldenMismatch => "native co-op component golden is invalid",
            Self::ChecksumMismatch => "native co-op component checksum is invalid",
            Self::ProducerDigestMismatch => {
                "native co-op producer digest does not match the component schema"
            }
            Self::Unadmitted => "native co-op component is unadmitted",
            Self::ArtifactTooLarge => "native co-op artifact exceeds its byte bound",
        })
    }
}

impl std::error::Error for CoopNativeArtifactError {}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CoopNativeArtifactLineage {
    artifact: String,
    protocol_version: String,
    schema_digest: String,
    producer_declared_schema_digest: String,
    status: CoopNativeArtifactStatus,
    admission: CoopNativeAdmissionStatus,
    provenance_source: String,
    provenance_generator: String,
    identities: CoopNativeLineage,
}

impl CoopNativeArtifactLineage {
    pub fn new(identities: CoopNativeLineage) -> Result<Self, CoopNativeArtifactError> {
        let state = verify_coop_native_artifact_state()?;
        Ok(Self {
            artifact: COOP_NATIVE_ARTIFACT.to_owned(),
            protocol_version: COOP_NATIVE_PROTOCOL_VERSION.to_owned(),
            schema_digest: COOP_NATIVE_SCHEMA_DIGEST.to_owned(),
            producer_declared_schema_digest: COOP_NATIVE_PRODUCER_SCHEMA_DIGEST.to_owned(),
            status: state.status,
            admission: state.admission,
            provenance_source: COOP_NATIVE_SCHEMA_SOURCE.to_owned(),
            provenance_generator: COOP_NATIVE_GENERATOR.to_owned(),
            identities,
        })
    }

    #[must_use]
    pub fn artifact(&self) -> &str {
        &self.artifact
    }

    #[must_use]
    pub fn protocol_version(&self) -> &str {
        &self.protocol_version
    }

    #[must_use]
    pub fn schema_digest(&self) -> &str {
        &self.schema_digest
    }

    #[must_use]
    pub fn producer_declared_schema_digest(&self) -> &str {
        &self.producer_declared_schema_digest
    }

    #[must_use]
    pub const fn is_admitted(&self) -> bool {
        matches!(
            self.admission,
            CoopNativeAdmissionStatus::Component | CoopNativeAdmissionStatus::Admitted
        )
    }

    #[must_use]
    pub const fn status(&self) -> CoopNativeArtifactStatus {
        self.status
    }

    #[must_use]
    pub const fn admission(&self) -> CoopNativeAdmissionStatus {
        self.admission
    }

    #[must_use]
    pub fn identities(&self) -> &CoopNativeLineage {
        &self.identities
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CoopNativeArtifactRecord {
    content_digest: String,
    byte_length: u64,
    lineage: CoopNativeArtifactLineage,
}

impl CoopNativeArtifactRecord {
    pub fn new(
        lineage: CoopNativeArtifactLineage,
        bytes: &[u8],
    ) -> Result<Self, CoopNativeArtifactError> {
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(CoopNativeArtifactError::ArtifactTooLarge);
        }
        Ok(Self {
            content_digest: format!("{:x}", Sha256::digest(bytes)),
            byte_length: bytes.len() as u64,
            lineage,
        })
    }

    #[must_use]
    pub fn content_digest(&self) -> &str {
        &self.content_digest
    }

    #[must_use]
    pub const fn byte_length(&self) -> u64 {
        self.byte_length
    }

    #[must_use]
    pub fn lineage(&self) -> &CoopNativeArtifactLineage {
        &self.lineage
    }
}

include!("coop_native_artifact_verify.rs");
