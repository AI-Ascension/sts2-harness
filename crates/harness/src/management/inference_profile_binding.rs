// SPDX-License-Identifier: MIT

//! Credential-free inference-profile bindings for an admitted definition.
//!
//! [`resolve_definition`] resolves every decide/planner reference of a workflow
//! against an authoritative catalog and seals the per-node requested/resolved
//! provenance by digest.  Only bounded metadata is retained: never a credential,
//! endpoint or tenant identifier.

use serde::{Deserialize, Serialize};

use super::contract::{InferenceProfileCatalog, InferenceProfileDescriptor};
use super::inference_profile_catalog::{
    INFERENCE_PROFILE_BINDINGS_SCHEMA_VERSION, INFERENCE_PROFILE_PROVENANCE_PREFIX,
};
use super::service::ManagementError;
use crate::sha256_hex;
use crate::workflow::{NodeDefinition, WorkflowDefinition, WorkflowLimits};

/// The authored reference together with the structural location it was found
/// at, so an admission refusal can name the offending node instead of only the
/// reference text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InferenceProfileReferenceSite {
    pub graph_id: String,
    pub node_id: String,
    pub node_kind: String,
    pub profile_ref: String,
    /// JSON path of the reference within the submitted definition, e.g.
    /// `$.graphs[0].nodes[1].config.planner_profile_ref`.
    pub path: String,
}

/// The exact revision one authored reference resolved to, with the site it was
/// resolved from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InferenceProfileResolution {
    pub site: InferenceProfileReferenceSite,
    pub profile_id: String,
    pub version: String,
    pub digest: String,
}

impl InferenceProfileResolution {
    /// The exact `profile_id:version:digest` identity a floating reference
    /// resolved to, in the same shape the definition itself may author.
    #[must_use]
    pub fn exact_pin(&self) -> String {
        format!("{}:{}:{}", self.profile_id, self.version, self.digest)
    }
}

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
    ///
    /// Informational, and deliberately not sealed: the selection is enforced
    /// against every binding's `adapter` before this set exists, so each binding
    /// already carries it. See `SealedBindings`.
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

/// The content a binding set seals: every resolved revision binding, and the
/// catalog revision they were resolved from.
///
/// The target-level selection is excluded on purpose. It is checked against
/// every binding's adapter before this set exists, so sealing it adds no
/// information — and it would make the execution-side fence unreproducible,
/// because that fence reads the durable admission, where the same field holds
/// this set's own reference rather than the consumer's selection.
#[derive(Serialize)]
struct SealedBindings<'a> {
    schema_version: &'a str,
    catalog_digest: &'a str,
    bindings: &'a [InferenceProfileBinding],
}

/// Resolves every decide/planner node of `definition` against `catalog`.
///
/// `selection` is the consumer's target-level adapter selection exactly as
/// submitted, or `None` for a fence that re-resolves a recorded admission and
/// must not treat the recorded provenance reference as a selection. Each call
/// site states which of the two it is: a resolution that inferred it from the
/// target could not tell them apart, and would silently drop a legitimate
/// selection that begins with the provenance prefix.
pub fn resolve_definition(
    catalog: &InferenceProfileCatalog,
    definition: &WorkflowDefinition,
    selection: Option<&str>,
) -> Result<InferenceProfileBindingSet, ManagementError> {
    let bindings = resolve_bindings(catalog, definition, selection)?
        .into_iter()
        .map(|(binding, _site)| binding)
        .collect();
    let mut set = InferenceProfileBindingSet {
        schema_version: INFERENCE_PROFILE_BINDINGS_SCHEMA_VERSION.to_owned(),
        catalog_digest: catalog.catalog_digest.clone(),
        target_inference_profile: selection.map(str::to_owned),
        bindings,
        digest: String::new(),
    };
    set.digest = seal(&set)?;
    Ok(set)
}

/// Resolves every decide/planner reference of `definition` and returns the
/// per-node resolved sites without sealing a binding set.
///
/// This is the same admission authority as [`resolve_definition`] — the same
/// catalog, the same per-node fences — exposed as the sites a caller needs to
/// *report* the decision: the authored reference, the node it came from, and the
/// exact revision it resolved to. It exists so definition validation and Studio
/// publication can publish the owner's resolution instead of each re-deriving
/// an admission rule.
///
/// `selection` is passed through to the identical fences used by
/// [`resolve_definition`]; see that function for why it is not inferred here.
pub fn resolve_definition_sites(
    catalog: &InferenceProfileCatalog,
    definition: &WorkflowDefinition,
    selection: Option<&str>,
) -> Result<Vec<InferenceProfileResolution>, ManagementError> {
    Ok(resolve_bindings(catalog, definition, selection)?
        .into_iter()
        .map(|(binding, site)| InferenceProfileResolution {
            site,
            profile_id: binding.profile_id,
            version: binding.version,
            digest: binding.digest,
        })
        .collect())
}

/// The single per-node admission walk both public projections read, so the
/// sealed binding set and the published resolution sites cannot drift apart:
/// there is exactly one ordering of the fences and one place a refusal is
/// raised.
fn resolve_bindings(
    catalog: &InferenceProfileCatalog,
    definition: &WorkflowDefinition,
    selection: Option<&str>,
) -> Result<Vec<(InferenceProfileBinding, InferenceProfileReferenceSite)>, ManagementError> {
    let mut resolutions = Vec::new();
    for (graph_index, graph) in definition.graphs.iter().enumerate() {
        for (node_index, node) in graph.nodes.iter().enumerate() {
            let Some((node_kind, reference, context_ref)) = inference_node_parts(node) else {
                continue;
            };
            let descriptor = catalog.resolve(reference, node_kind)?;
            admit_target(descriptor, selection)?;
            admit_limits(descriptor, &definition.limits)?;
            admit_context(descriptor, context_ref)?;
            let site = InferenceProfileReferenceSite {
                graph_id: graph.id.as_str().to_owned(),
                node_id: node.id().as_str().to_owned(),
                node_kind: node_kind.to_owned(),
                profile_ref: reference.to_owned(),
                path: format!(
                    "$.graphs[{graph_index}].nodes[{node_index}].config.{}",
                    reference_field(node_kind)
                ),
            };
            resolutions.push((
                InferenceProfileBinding {
                    graph_id: site.graph_id.clone(),
                    node_id: site.node_id.clone(),
                    node_kind: node_kind.to_owned(),
                    profile_ref: reference.to_owned(),
                    profile_id: descriptor.profile_id.clone(),
                    version: descriptor.version.clone(),
                    digest: descriptor.digest.clone(),
                    adapter: descriptor.adapter.clone(),
                    requested_model: descriptor.requested_model.clone(),
                    resolved_model: descriptor.resolved_model.clone(),
                },
                site,
            ));
        }
    }
    Ok(resolutions)
}

/// The config field that carries a node kind's inference-profile reference.
fn reference_field(node_kind: &str) -> &'static str {
    match node_kind {
        "decide" => "decision_profile_ref",
        _ => "planner_profile_ref",
    }
}

/// Seals `set` by hashing its sealed content.
fn seal(set: &InferenceProfileBindingSet) -> Result<String, ManagementError> {
    let sealed = SealedBindings {
        schema_version: &set.schema_version,
        catalog_digest: &set.catalog_digest,
        bindings: &set.bindings,
    };
    let bytes = serde_json::to_vec(&sealed).map_err(|error| {
        ManagementError::invalid("inference_profile_bindings_encode", error.to_string())
    })?;
    Ok(sha256_hex(bytes))
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
