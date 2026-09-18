// SPDX-License-Identifier: MIT

//! Credential-free inference-profile bindings for an admitted definition.
//!
//! [`resolve_definition`] resolves every decide/planner reference of a workflow
//! against an authoritative catalog and seals the per-node requested/resolved
//! provenance by digest.  Only bounded metadata is retained: never a credential,
//! endpoint or tenant identifier.

use serde::{Deserialize, Serialize};

use super::contract::{
    InferenceProfileCatalog, InferenceProfileDescriptor, RunTargetConfiguration,
};
use super::inference_profile_catalog::{
    INFERENCE_PROFILE_BINDINGS_SCHEMA_VERSION, INFERENCE_PROFILE_PROVENANCE_PREFIX,
};
use super::service::ManagementError;
use crate::sha256_hex;
use crate::workflow::{NodeDefinition, WorkflowDefinition, WorkflowLimits};

/// One node's requested reference and the exact revision it resolved to.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InferenceProfileBinding {
    pub graph_id: String,
    pub node_id: String,
    pub node_kind: String,
    pub profile_ref: String,
    pub profile_id: String,
    pub version: String,
    pub digest: String,
    pub adapter: String,
    pub requested_model: String,
    pub resolved_model: Option<String>,
}

/// Every inference binding of one admitted definition, sealed by digest.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InferenceProfileBindingSet {
    pub schema_version: String,
    pub catalog_digest: String,
    /// The target-level adapter the consumer selected at preflight, if any.
    pub target_inference_profile: Option<String>,
    pub bindings: Vec<InferenceProfileBinding>,
    pub digest: String,
}

impl InferenceProfileBindingSet {
    /// The credential-free provenance reference persisted on the run record.
    #[must_use]
    pub fn reference(&self) -> String {
        format!("{INFERENCE_PROFILE_PROVENANCE_PREFIX}{}", self.digest)
    }
}

/// Resolves every decide/planner node of `definition` against `catalog`.
pub fn resolve_definition(
    catalog: &InferenceProfileCatalog,
    definition: &WorkflowDefinition,
    target: &RunTargetConfiguration,
) -> Result<InferenceProfileBindingSet, ManagementError> {
    // A recorded provenance reference is the output of a previous resolution,
    // not a consumer's target-level selection; the second fence re-resolves
    // from the same inputs the first one used.
    let selection = target
        .inference_profile
        .as_deref()
        .filter(|value| !value.starts_with(INFERENCE_PROFILE_PROVENANCE_PREFIX));
    let mut bindings = Vec::new();
    for graph in &definition.graphs {
        for node in &graph.nodes {
            let Some((node_kind, reference, context_ref)) = inference_node_parts(node) else {
                continue;
            };
            let descriptor = catalog.resolve(reference, node_kind)?;
            admit_target(descriptor, selection)?;
            admit_limits(descriptor, &definition.limits)?;
            admit_context(descriptor, context_ref)?;
            bindings.push(InferenceProfileBinding {
                graph_id: graph.id.as_str().to_owned(),
                node_id: node.id().as_str().to_owned(),
                node_kind: node_kind.to_owned(),
                profile_ref: reference.to_owned(),
                profile_id: descriptor.profile_id.clone(),
                version: descriptor.version.clone(),
                digest: descriptor.digest.clone(),
                adapter: descriptor.adapter.clone(),
                requested_model: descriptor.requested_model.clone(),
                resolved_model: descriptor.resolved_model.clone(),
            });
        }
    }
    let mut set = InferenceProfileBindingSet {
        schema_version: INFERENCE_PROFILE_BINDINGS_SCHEMA_VERSION.to_owned(),
        catalog_digest: catalog.catalog_digest.clone(),
        target_inference_profile: selection.map(str::to_owned),
        bindings,
        digest: String::new(),
    };
    let bytes = serde_json::to_vec(&set).map_err(|error| {
        ManagementError::invalid("inference_profile_bindings_encode", error.to_string())
    })?;
    set.digest = sha256_hex(bytes);
    Ok(set)
}

/// `(node_kind, profile reference, context reference)` for inference nodes.
fn inference_node_parts(node: &NodeDefinition) -> Option<(&str, &str, Option<&str>)> {
    match node {
        NodeDefinition::Decide { config, .. } => Some((
            "decide",
            config.decision_profile_ref.as_str(),
            Some(config.context_ref.as_str()),
        )),
        NodeDefinition::AdaptiveRegion { config, .. } => {
            Some(("adaptive_region", config.planner_profile_ref.as_str(), None))
        }
        _ => None,
    }
}

fn admit_target(
    descriptor: &InferenceProfileDescriptor,
    selection: Option<&str>,
) -> Result<(), ManagementError> {
    if let Some(adapter) = selection
        && descriptor.adapter != adapter
    {
        return Err(ManagementError::capability(
            "inference_profile_adapter_mismatch",
            "the resolved inference profile is served by a different adapter than the admitted target selection",
        ));
    }
    Ok(())
}

fn admit_limits(
    descriptor: &InferenceProfileDescriptor,
    limits: &WorkflowLimits,
) -> Result<(), ManagementError> {
    let budgets = &descriptor.effective_budgets;
    if limits.max_provider_calls > budgets.max_provider_calls
        || limits.max_output_tokens > budgets.max_output_tokens
    {
        return Err(ManagementError::capability(
            "inference_profile_budget_exceeded",
            "workflow limits exceed the inference profile's effective budgets",
        ));
    }
    Ok(())
}

fn admit_context(
    descriptor: &InferenceProfileDescriptor,
    context_ref: Option<&str>,
) -> Result<(), ManagementError> {
    if let Some(context_ref) = context_ref
        && !descriptor.context_compatibility.is_empty()
        && !descriptor
            .context_compatibility
            .iter()
            .any(|value| value == context_ref)
    {
        return Err(ManagementError::capability(
            "inference_profile_context_incompatible",
            "the inference profile does not accept this node's context reference",
        ));
    }
    Ok(())
}
