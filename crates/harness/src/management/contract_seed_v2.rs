// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::auth::AuthContext;
use super::contract::{
    MANAGEMENT_SCHEMA_VERSION, RunRequest, RunSubmissionResponse, TargetAdmissionBinding,
};
use crate::seed_binding::{SeedBindingError, canonicalize_seed};

pub const WORKFLOW_RUN_REQUEST_V2_SCHEMA: &str = "ascension.workflow-run-request/v2";
pub const WORKFLOW_SEED_REQUEST_V2_SCHEMA: &str = "ascension.workflow-seed-request/v2";
pub const WORKFLOW_SEED_BINDING_V2_SCHEMA: &str = "ascension.workflow-seed-binding/v2";
pub(crate) const WORKFLOW_SEED_OPERATION_V2_SCHEMA: &str = "ascension.workflow-seed-operation/v2";
pub const WORKFLOW_RUN_SUBMISSION_V2_SCHEMA: &str = "ascension.workflow-run-submission/v2";
pub const SEED_DERIVATION_ALGORITHM_V1: &str = "hmac-sha256-v1";

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeedModeV2 {
    Explicit,
    DeriveOnce,
}

/// A closed, additive seed request nested only under the v2 workflow request.
///
/// `seed` is present exactly for `explicit`; `derive_once` contains no caller
/// supplied seed or key-selection field.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SeedRequestV2 {
    pub schema_version: String,
    pub mode: SeedModeV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<String>,
}

impl SeedRequestV2 {
    pub fn validate(&self) -> Result<(), SeedBindingError> {
        if self.schema_version != WORKFLOW_SEED_REQUEST_V2_SCHEMA {
            return Err(SeedBindingError::ConfigurationConflict);
        }
        match (self.mode, self.seed.as_deref()) {
            (SeedModeV2::Explicit, Some(seed)) => canonicalize_seed(seed).map(|_| ()),
            (SeedModeV2::DeriveOnce, None) => Ok(()),
            _ => Err(SeedBindingError::ConfigurationConflict),
        }
    }
}

/// Closed v2 request for actor-bound, durable seed arbitration.
///
/// The existing `RunRequest` remains unchanged and is used only as an internal
/// execution projection after the v2 digest and seed record are prepared.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowRunRequestV2 {
    pub schema_version: String,
    pub request_id: String,
    pub definition: Option<Value>,
    pub artifact_id: Option<String>,
    pub instance_id: String,
    pub profile: String,
    pub admission: Option<TargetAdmissionBinding>,
    pub seed: SeedRequestV2,
}

impl WorkflowRunRequestV2 {
    pub fn into_execution_request(self) -> RunRequest {
        RunRequest {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            request_id: self.request_id,
            definition: self.definition,
            artifact_id: self.artifact_id,
            instance_id: self.instance_id,
            profile: self.profile,
            admission: self.admission,
        }
    }

    pub fn validate_seed(&self) -> Result<(), SeedBindingError> {
        if self.schema_version != WORKFLOW_RUN_REQUEST_V2_SCHEMA {
            return Err(SeedBindingError::ConfigurationConflict);
        }
        self.seed.validate()
    }
}

/// Public view of a persisted seed binding. It deliberately omits the actor
/// digest, request digest, and key-material commitment held by the store.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SeedBindingReadbackV2 {
    pub schema_version: String,
    pub workflow_run_id: String,
    pub operation_id: String,
    pub mode: SeedModeV2,
    pub requested_seed: Option<String>,
    pub effective_seed: String,
    pub algorithm_id: Option<String>,
    pub key_authority_id: Option<String>,
    pub key_version: Option<String>,
    pub configuration_digest: String,
    pub state: SeedBindingStateV2,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeedBindingStateV2 {
    CandidatePersisted,
    AwaitingHostContext,
    Resolved,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SeededRunSubmissionResponseV2 {
    pub schema_version: String,
    pub run: RunSubmissionResponse,
    pub seed_binding: SeedBindingReadbackV2,
}

/// Internal durable choice of the derivation identity before the effective
/// seed is calculated. This record is never serialized onto the public wire.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSeedOperationV2 {
    pub schema_version: String,
    pub request_id: String,
    pub actor_digest: String,
    pub request_digest: String,
    pub workflow_run_id: String,
    pub operation_id: String,
    pub mode: SeedModeV2,
    pub admitted_configuration: TargetAdmissionBinding,
    pub configuration_digest: String,
    pub derivation: StoredSeedDerivationV2,
    pub phase: SeedOperationPhaseV2,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SeedOperationPhaseV2 {
    KeyPinned,
    CandidatePersisted,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSeedDerivationV2 {
    pub algorithm_id: String,
    pub key: super::seed_key::SeedKeyIdentity,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSeedBindingV2 {
    pub schema_version: String,
    pub request_id: String,
    pub actor_digest: String,
    pub request_digest: String,
    pub workflow_run_id: String,
    pub operation_id: String,
    pub mode: SeedModeV2,
    pub requested_seed: Option<String>,
    pub effective_seed: String,
    pub derivation: Option<StoredSeedDerivationV2>,
    /// The exact owner-revalidated admission captured with the first durable
    /// reservation. Run snapshots can evolve after commands; this copy is the
    /// immutable configuration authority for replaying the seed operation.
    pub admitted_configuration: TargetAdmissionBinding,
    pub configuration_digest: String,
    pub state: SeedBindingStateV2,
}

impl std::fmt::Debug for StoredSeedBindingV2 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredSeedBindingV2")
            .field("schema_version", &self.schema_version)
            .field("request_id", &self.request_id)
            .field("actor_digest", &"[redacted]")
            .field("request_digest", &self.request_digest)
            .field("workflow_run_id", &self.workflow_run_id)
            .field("operation_id", &self.operation_id)
            .field("mode", &"[redacted]")
            .field("requested_seed", &"[redacted]")
            .field("effective_seed", &"[redacted]")
            .field(
                "derivation",
                &self.derivation.as_ref().map(|pin| &pin.algorithm_id),
            )
            .field("admitted_configuration", &"[redacted]")
            .field("configuration_digest", &self.configuration_digest)
            .field("state", &"[redacted]")
            .finish()
    }
}

impl StoredSeedBindingV2 {
    pub(crate) fn readback(&self) -> SeedBindingReadbackV2 {
        SeedBindingReadbackV2 {
            schema_version: WORKFLOW_SEED_BINDING_V2_SCHEMA.to_owned(),
            workflow_run_id: self.workflow_run_id.to_owned(),
            operation_id: self.operation_id.to_owned(),
            mode: self.mode,
            requested_seed: self.requested_seed.clone(),
            effective_seed: self.effective_seed.clone(),
            algorithm_id: self.derivation.as_ref().map(|pin| pin.algorithm_id.clone()),
            key_authority_id: self
                .derivation
                .as_ref()
                .map(|pin| pin.key.authority_id.clone()),
            key_version: self.derivation.as_ref().map(|pin| pin.key.version.clone()),
            configuration_digest: self.configuration_digest.to_owned(),
            state: self.state,
        }
    }

    pub(crate) fn visible_to(&self, actor: &AuthContext) -> bool {
        super::seed_v2_crypto::actor_digest(&actor.subject)
            .is_ok_and(|digest| digest == self.actor_digest)
    }
}

#[cfg(test)]
#[path = "contract_seed_v2_tests.rs"]
mod tests;
