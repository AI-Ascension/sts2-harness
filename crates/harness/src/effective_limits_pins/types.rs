// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

/// Schema of the committed producer/consumer pin and digest conformance matrix.
pub const EFFECTIVE_LIMIT_PIN_MATRIX_SCHEMA: &str =
    "ascension.harness.effective-limit-pin-matrix.v1";

pub const MEMORY_POLICY_SCHEMA_PATH: &str = "contracts/context-memory/policy.schema.json";
pub const MEMORY_CAPABILITIES_SCHEMA_PATH: &str =
    "contracts/context-memory/capabilities.schema.json";
pub const SESSION_POLICY_SCHEMA_PATH: &str = "contracts/provider-session/policy.schema.json";
pub const SESSION_CAPABILITIES_SCHEMA_PATH: &str =
    "contracts/provider-session/capabilities.schema.json";
pub const STUDIO_CONTRACT_WORKFLOW_PATH: &str = ".github/workflows/studio-contract.yml";

const MATRIX_JSON: &str = include_str!("../../../../contracts/effective-limits-pins.json");
const MEMORY_POLICY_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../contracts/context-memory/policy.schema.json");
const MEMORY_CAPABILITIES_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../contracts/context-memory/capabilities.schema.json");
const SESSION_POLICY_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../contracts/provider-session/policy.schema.json");
const SESSION_CAPABILITIES_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../contracts/provider-session/capabilities.schema.json");
const STUDIO_CONTRACT_WORKFLOW: &str =
    include_str!("../../../../.github/workflows/studio-contract.yml");

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Adoption {
    /// The consumer validates the current capability revision and discloses effective limits.
    Aligned,
    /// The consumer cannot yet discover executable ceilings, so every value stays unavailable.
    Pending,
}

/// How the consumer obtains the owner contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactMode {
    /// The consumer copies owner contract bytes, so its digests must equal the producer digests.
    CopiedContracts,
    /// The consumer implements the schema natively and only records digests for provenance.
    SchemaAdapter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionSource {
    /// The revision the harness CI lane checks out.
    HarnessCiPin,
    /// The consumer head observed while recording this matrix.
    ObservedHead,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProducerArtifact {
    pub kind: String,
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProducerSurface {
    pub surface: String,
    pub capability_schema: String,
    pub capability_revision: String,
    pub artifacts: Vec<ProducerArtifact>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProducerPins {
    pub repository: String,
    /// Descriptor owner identity, distinct from the repository name.
    pub owner: String,
    pub surfaces: Vec<ProducerSurface>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessCiPin {
    pub workflow: String,
    pub revision: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumerArtifact {
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumerSurface {
    pub surface: String,
    /// Capability schema the consumer copy validates, when it validates one at all.
    pub advertised_capability_schema: Option<String>,
    /// Whether the consumer can discover executable ceilings from its copy.
    pub effective_limits_advertised: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumerPin {
    pub repository: String,
    pub revision: String,
    pub revision_source: RevisionSource,
    pub artifact_mode: ArtifactMode,
    pub role: String,
    pub adoption: Adoption,
    pub artifacts: Vec<ConsumerArtifact>,
    pub surfaces: Vec<ConsumerSurface>,
    pub harness_ci_pin: Option<HarnessCiPin>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinMatrix {
    pub schema: String,
    pub recorded_on: String,
    pub producer: ProducerPins,
    pub consumers: Vec<ConsumerPin>,
}

pub(super) fn matrix_json() -> &'static str {
    MATRIX_JSON
}

pub(super) fn producer_artifact_bytes(path: &str) -> Option<&'static [u8]> {
    match path {
        MEMORY_POLICY_SCHEMA_PATH => Some(MEMORY_POLICY_SCHEMA_BYTES),
        MEMORY_CAPABILITIES_SCHEMA_PATH => Some(MEMORY_CAPABILITIES_SCHEMA_BYTES),
        SESSION_POLICY_SCHEMA_PATH => Some(SESSION_POLICY_SCHEMA_BYTES),
        SESSION_CAPABILITIES_SCHEMA_PATH => Some(SESSION_CAPABILITIES_SCHEMA_BYTES),
        _ => None,
    }
}

pub(super) fn workflow_pin_matches(workflow: &str, revision: &str) -> bool {
    match workflow {
        STUDIO_CONTRACT_WORKFLOW_PATH => {
            STUDIO_CONTRACT_WORKFLOW.contains(&format!("ref: {revision}"))
        }
        _ => false,
    }
}

pub(super) fn valid_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
}

pub(super) fn valid_revision(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
