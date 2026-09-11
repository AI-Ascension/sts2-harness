// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use super::ids::{
    CapabilityId, FieldId, GraphId, GuardId, NodeId, OutputId, PolicyId, ProfileId,
    SemanticVersion, WorkflowId,
};

pub use super::node::*;

pub const WORKFLOW_SCHEMA_VERSION: &str = "ascension.workflow/v1";

/// Authoritative compiler identity for the harness workflow compiler/validator.
/// It is reported separately from a definition digest or a Studio layout digest.
pub const WORKFLOW_COMPILER_ID: &str = "sts2-harness.workflow-compiler.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowSchemaVersion {
    #[serde(rename = "ascension.workflow/v1")]
    V1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowMode {
    Strict,
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCapabilities {
    pub required: Vec<CapabilityId>,
    pub optional: Vec<CapabilityId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowLimits {
    pub max_steps: u64,
    pub max_subworkflow_depth: u64,
    pub max_provider_calls: u64,
    pub max_parallel_analyses: u64,
    pub max_output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Annotations {
    pub summary: String,
    pub synthetic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDefinition {
    pub schema_version: WorkflowSchemaVersion,
    pub workflow_id: WorkflowId,
    pub version: SemanticVersion,
    pub mode: WorkflowMode,
    pub game_profile: ProfileId,
    pub policy_ref: PolicyId,
    pub capabilities: WorkflowCapabilities,
    pub limits: WorkflowLimits,
    pub entry_graph: GraphId,
    pub graphs: Vec<GraphDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Annotations>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphDefinition {
    pub id: GraphId,
    pub entry_node: NodeId,
    pub nodes: Vec<NodeDefinition>,
    pub edges: Vec<ControlEdge>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guards: Vec<GuardDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlEdge {
    pub from: NodeId,
    pub to: NodeId,
    pub on: EdgeOutcome,
    pub priority: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard_ref: Option<GuardId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeOutcome {
    Ok,
    Error,
    Timeout,
    Unavailable,
    True,
    False,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataBinding {
    pub destination_node: NodeId,
    pub destination_input: OutputId,
    pub source: BindingSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BindingSource {
    GraphInput { graph_input: FieldId },
    NodeOutput { node_id: NodeId, output: OutputId },
}
