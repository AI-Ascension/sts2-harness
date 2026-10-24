// SPDX-License-Identifier: MIT

use crate::identity::ModelExecutionId;
use crate::recipe::contract_v2::{AdmittedRecipeV2, RecipeOperationV2};

use super::core::{ExecutionLineage, valid_digest, valid_id};
use super::error::ExecutionStoreError;

pub(crate) const MAP_RECIPE_ID: &str = "runtime.map-context";
pub(crate) const MAP_RECIPE_REVISION: i64 = 1;
pub(crate) const MAP_RECIPE_OPERATION: &str = "map_snapshot";
pub(crate) const MAP_RPC_CORRELATION_ID: &str = "3";
pub(crate) const MAP_RECEIPT_PROFILE: &str = "runtime-map-v1";
const MAX_MAP_IDENTITY: u64 = 9_007_199_254_740_991;

/// The runtime-owned identities and observation fence for one fixed map read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipeInvocationContext {
    /// Digest of the admitted runtime configuration. Credentials are not included.
    pub runtime_config_digest: String,
    /// Gateway-owned instance identity.
    pub instance_id: String,
    /// Gateway session identity returned in the map response.
    pub gateway_session_id: String,
    /// MCP session identity sent as a request argument.
    pub mcp_session_id: String,
    /// Gateway lease identity.
    pub lease_id: String,
    /// Gateway lease epoch.
    pub lease_epoch: u64,
    /// State identity from the already-admitted observation.
    pub state_id: String,
    /// Generation from the already-admitted observation.
    pub generation: u64,
}

/// Durable state of one fixed map invocation receipt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecipeInvocationStatus {
    /// The immutable request fence was committed before the map read.
    IntentRecorded,
    /// The protocol response digest was recorded after the read.
    ResponseValidated,
    /// The response matched the owner-accepted decision context.
    ContextValidated,
}

impl RecipeInvocationStatus {
    pub(crate) fn parse(value: &str) -> Result<Self, ExecutionStoreError> {
        match value {
            "intent_recorded" => Ok(Self::IntentRecorded),
            "response_validated" => Ok(Self::ResponseValidated),
            "context_validated" => Ok(Self::ContextValidated),
            _ => Err(ExecutionStoreError::Corrupt),
        }
    }
}

/// Read-only receipt metadata. It never contains a map snapshot or other payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipeInvocationReceipt {
    status: RecipeInvocationStatus,
    request_digest: String,
    typed_result_digest: Option<String>,
    owner_snapshot_digest: Option<String>,
    decision_input_digest: Option<String>,
}

impl RecipeInvocationReceipt {
    pub(crate) fn new(
        status: RecipeInvocationStatus,
        request_digest: String,
        typed_result_digest: Option<String>,
        owner_snapshot_digest: Option<String>,
        decision_input_digest: Option<String>,
    ) -> Self {
        Self {
            status,
            request_digest,
            typed_result_digest,
            owner_snapshot_digest,
            decision_input_digest,
        }
    }

    /// Returns the immutable receipt state.
    #[must_use]
    pub const fn status(&self) -> RecipeInvocationStatus {
        self.status
    }

    /// Returns the digest of the exact invocation binding.
    #[must_use]
    pub fn request_digest(&self) -> &str {
        &self.request_digest
    }

    /// Returns the canonical typed response digest when a response was recorded.
    #[must_use]
    pub fn typed_result_digest(&self) -> Option<&str> {
        self.typed_result_digest.as_deref()
    }

    /// Returns the owner-accepted snapshot digest after context validation.
    #[must_use]
    pub fn owner_snapshot_digest(&self) -> Option<&str> {
        self.owner_snapshot_digest.as_deref()
    }

    /// Returns the effective decision fingerprint after context validation.
    #[must_use]
    pub fn decision_input_digest(&self) -> Option<&str> {
        self.decision_input_digest.as_deref()
    }
}

/// Immutable fence for the one admitted runtime-owned V2 map invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipeInvocationBinding {
    pub(crate) lineage: ExecutionLineage,
    pub(crate) model_execution_id: String,
    pub(crate) context: RecipeInvocationContext,
}

impl RecipeInvocationBinding {
    /// Creates a binding only for the fixed zero-argument map recipe.
    ///
    /// # Errors
    ///
    /// Returns an error when the recipe or any identity/fence value is outside its bound.
    pub fn new(
        lineage: ExecutionLineage,
        model_execution_id: ModelExecutionId,
        recipe: &AdmittedRecipeV2,
        context: RecipeInvocationContext,
    ) -> Result<Self, ExecutionStoreError> {
        if recipe.id().as_str() != MAP_RECIPE_ID
            || i64::from(recipe.revision().get()) != MAP_RECIPE_REVISION
            || recipe.operation() != (RecipeOperationV2::MapSnapshot {})
        {
            return Err(ExecutionStoreError::InvalidOperation);
        }
        let binding = Self {
            lineage,
            model_execution_id: model_execution_id.to_string(),
            context,
        };
        binding.validate()?;
        Ok(binding)
    }

    /// Returns the execution lineage bound to this invocation.
    #[must_use]
    pub fn lineage(&self) -> &ExecutionLineage {
        &self.lineage
    }

    /// Returns the model-execution identity in its own namespace.
    #[must_use]
    pub fn model_execution_id(&self) -> &str {
        &self.model_execution_id
    }

    /// Returns the immutable runtime and observation fence.
    #[must_use]
    pub fn context(&self) -> &RecipeInvocationContext {
        &self.context
    }

    pub(crate) fn validate(&self) -> Result<(), ExecutionStoreError> {
        self.lineage
            .validate()
            .map_err(|_| ExecutionStoreError::InvalidOperation)?;
        let context = &self.context;
        if !valid_id(&self.model_execution_id)
            || !valid_digest(&context.runtime_config_digest)
            || !valid_id(&context.instance_id)
            || !valid_id(&context.gateway_session_id)
            || !valid_id(&context.mcp_session_id)
            || !valid_id(&context.lease_id)
            || !valid_id(&context.state_id)
            || context.lease_epoch > MAX_MAP_IDENTITY
            || context.generation > MAX_MAP_IDENTITY
        {
            return Err(ExecutionStoreError::InvalidOperation);
        }
        Ok(())
    }

    pub(crate) fn request_digest(&self) -> String {
        let revision = MAP_RECIPE_REVISION.to_string();
        let lease_epoch = self.context.lease_epoch.to_string();
        let generation = self.context.generation.to_string();
        let fields = [
            self.lineage.run_id.as_str(),
            self.lineage.episode_id.as_str(),
            self.lineage.attempt_id.as_str(),
            self.lineage.trajectory_id.as_str(),
            self.model_execution_id.as_str(),
            MAP_RECIPE_ID,
            revision.as_str(),
            MAP_RECIPE_OPERATION,
            self.context.runtime_config_digest.as_str(),
            self.context.instance_id.as_str(),
            self.context.gateway_session_id.as_str(),
            self.context.mcp_session_id.as_str(),
            self.context.lease_id.as_str(),
            lease_epoch.as_str(),
            self.context.state_id.as_str(),
            generation.as_str(),
            MAP_RECEIPT_PROFILE,
            crate::RUNTIME_MAP_SCHEMA_DIGEST,
            MAP_RPC_CORRELATION_ID,
        ];
        let mut material = Vec::with_capacity(512);
        material.extend_from_slice(b"sts2-harness:recipe-map-invocation:v1\0");
        for field in fields {
            material.extend_from_slice(field.as_bytes());
            material.push(0);
        }
        crate::sha256_hex(material)
    }
}
