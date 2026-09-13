// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use serde_json::Value;

use super::super::contract::{Diagnostic, DiagnosticSeverity, TargetCatalogResponse};
use super::super::service::{
    CapabilityPort, DefinitionPort, DiffResult, InspectionResult, ManagementError, ValidationResult,
};
use super::session::LiveWorkflowSessionFactory;
use crate::management::AuthContext;
use crate::workflow::{NodeDefinition, NodeKind, WorkflowDefinition};
use std::sync::Arc;

/// Capability advertised by an authoritative live session factory.
pub const LIVE_WORKFLOW_CAPABILITY: &str = "workflow.live";
/// Initial live management profile.
pub const LIVE_WORKFLOW_PROFILE: &str = "live.workflow.v1";

const NODE_CAPABILITIES: &[(&str, &str)] = &[
    ("observe", "workflow.node.observe.v1"),
    ("decide", "workflow.node.decide.v1"),
    ("execute_action", "workflow.node.execute_action.v1"),
    ("terminal", "workflow.node.terminal.v1"),
];

pub(super) fn validate_capability_manifest(value: &Value) -> Result<(), ManagementError> {
    let entries = value
        .get("capabilities")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ManagementError::invalid("capability_manifest", "capabilities must be an array")
        })?;
    if entries.len() > 128
        || entries.iter().any(|entry| {
            entry
                .as_str()
                .is_none_or(|item| item.is_empty() || item.len() > 128)
        })
    {
        return Err(ManagementError::invalid(
            "capability_manifest",
            "capability entries are outside their bounds",
        ));
    }
    Ok(())
}

/// Definition validation which adds node-level live admission diagnostics to
/// the common schema/capability checks.
pub(super) struct LiveDefinitionPort {
    capabilities: Value,
}

impl LiveDefinitionPort {
    pub(super) fn new(capabilities: Value) -> Result<Self, ManagementError> {
        validate_capability_manifest(&capabilities)?;
        Ok(Self { capabilities })
    }
}

impl DefinitionPort for LiveDefinitionPort {
    fn validate(
        &self,
        definition: &Value,
        capabilities: &Value,
    ) -> Result<ValidationResult, ManagementError> {
        let base = super::super::workflow_ports::SyntheticDefinitionPort;
        let mut result = DefinitionPort::validate(&base, definition, capabilities)?;
        let parsed = super::super::workflow_ports::parse_definition(definition)?;
        result
            .diagnostics
            .extend(node_diagnostics(&parsed, &self.capabilities));
        Ok(result)
    }

    fn inspect(&self, definition: &Value) -> Result<InspectionResult, ManagementError> {
        DefinitionPort::inspect(
            &super::super::workflow_ports::SyntheticDefinitionPort,
            definition,
        )
    }

    fn diff(
        &self,
        old_definition: &Value,
        new_definition: &Value,
    ) -> Result<DiffResult, ManagementError> {
        DefinitionPort::diff(
            &super::super::workflow_ports::SyntheticDefinitionPort,
            old_definition,
            new_definition,
        )
    }
}

fn node_diagnostics(definition: &WorkflowDefinition, capabilities: &Value) -> Vec<Diagnostic> {
    let available = capabilities
        .get("capabilities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    let mut diagnostics = Vec::new();
    for graph in &definition.graphs {
        for node in &graph.nodes {
            let kind = node_kind_name(node.kind());
            let Some(required) = NODE_CAPABILITIES
                .iter()
                .find_map(|(name, capability)| (*name == kind).then_some(*capability))
            else {
                diagnostics.push(Diagnostic {
                    code: "node_capability_unavailable".to_owned(),
                    severity: DiagnosticSeverity::Error,
                    path: format!("$.graphs.{}.nodes.{}", graph.id, node.id()),
                    message: format!("live execution does not support {kind} nodes"),
                });
                continue;
            };
            if !available.contains(required) {
                diagnostics.push(Diagnostic {
                    code: "node_capability_unavailable".to_owned(),
                    severity: DiagnosticSeverity::Error,
                    path: format!("$.graphs.{}.nodes.{}", graph.id, node.id()),
                    message: format!("required live node capability {required} is unavailable"),
                });
            }
            let path = format!("$.graphs.{}.nodes.{}.config", graph.id, node.id());
            match node {
                NodeDefinition::Observe { config, .. } => {
                    binding_diagnostic(
                        &mut diagnostics,
                        &available,
                        "workflow.projection.",
                        config.projection_ref.as_str(),
                        &format!("{path}.projection_ref"),
                    );
                }
                NodeDefinition::Decide { config, .. } => {
                    binding_diagnostic(
                        &mut diagnostics,
                        &available,
                        "workflow.provider.",
                        config.decision_profile_ref.as_str(),
                        &format!("{path}.decision_profile_ref"),
                    );
                    binding_diagnostic(
                        &mut diagnostics,
                        &available,
                        "workflow.context.",
                        config.context_ref.as_str(),
                        &format!("{path}.context_ref"),
                    );
                }
                _ => {}
            }
        }
    }
    if definition.game_profile.as_str().contains("synthetic")
        || definition.policy_ref.as_str().contains("synthetic")
    {
        diagnostics.push(Diagnostic {
            code: "live_binding_unavailable".to_owned(),
            severity: DiagnosticSeverity::Error,
            path: "$.game_profile".to_owned(),
            message: "live execution cannot use synthetic profile or policy bindings".to_owned(),
        });
    }
    diagnostics
}

fn binding_diagnostic(
    diagnostics: &mut Vec<Diagnostic>,
    available: &BTreeSet<&str>,
    prefix: &str,
    reference: &str,
    path: &str,
) {
    let capability = format!("{prefix}{reference}");
    if reference.contains("synthetic")
        || (!available.contains(capability.as_str()) && !available.contains(reference))
    {
        diagnostics.push(Diagnostic {
            code: "live_binding_unavailable".to_owned(),
            severity: DiagnosticSeverity::Error,
            path: path.to_owned(),
            message: format!("live binding {reference} is unavailable"),
        });
    }
}

fn node_kind_name(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Observe => "observe",
        NodeKind::Decide => "decide",
        NodeKind::ExecuteAction => "execute_action",
        NodeKind::Terminal => "terminal",
        NodeKind::AwaitStability => "await_stability",
        NodeKind::Route => "route",
        NodeKind::Analyze => "analyze",
        NodeKind::AdaptiveRegion => "adaptive_region",
        NodeKind::Subworkflow => "subworkflow",
        NodeKind::Loop => "loop",
        NodeKind::Checkpoint => "checkpoint",
        NodeKind::EmitArtifact => "emit_artifact",
        NodeKind::Pause => "pause",
    }
}

pub(super) struct LiveCapabilityPort {
    pub(super) capabilities: Value,
    pub(super) factory: Arc<dyn LiveWorkflowSessionFactory>,
}

impl CapabilityPort for LiveCapabilityPort {
    fn capabilities(&self) -> Result<Value, ManagementError> {
        Ok(self.capabilities.clone())
    }

    fn target_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        self.factory.target_catalog(actor)
    }
}
