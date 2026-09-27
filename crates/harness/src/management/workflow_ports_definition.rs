// SPDX-License-Identifier: MIT

//! The synthetic definition port: admission, validation, diff, and the diagnostics it reports,
//! plus the shared definition parse/digest helpers both execution ports use.
//!
//! Part of the `workflow_ports` split. Refs sts2-harness#570.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::contract::{Diagnostic, DiagnosticSeverity, validate_identifier};
use super::service::{
    DefinitionPort, DiffResult, InspectionResult, ManagementError, ValidationResult,
};
use crate::workflow::{
    DecodeError, WorkflowDefinition, decode_strict, semantic_diff, validate_definition,
};

pub(super) struct SyntheticDefinitionPort;

impl DefinitionPort for SyntheticDefinitionPort {
    fn validate(
        &self,
        definition: &Value,
        capabilities: &Value,
    ) -> Result<ValidationResult, ManagementError> {
        let parsed = parse_definition(definition)?;
        let digest = raw_digest(definition)?;
        let mut diagnostics = Vec::new();
        diagnostics.extend(capability_diagnostics(&parsed, capabilities)?);
        diagnostics.extend(context_reference_diagnostics(&parsed, capabilities)?);
        Ok(ValidationResult {
            definition_digest: digest,
            compiler: crate::workflow::WORKFLOW_COMPILER_ID.to_owned(),
            diagnostics,
        })
    }

    fn inspect(&self, definition: &Value) -> Result<InspectionResult, ManagementError> {
        let parsed = parse_definition(definition)?;
        Ok(InspectionResult {
            definition_digest: raw_digest(definition)?,
            workflow_id: Some(parsed.workflow_id.as_str().to_owned()),
            workflow_version: Some(parsed.version.as_str().to_owned()),
            required_capabilities: parsed
                .capabilities
                .required
                .iter()
                .map(|value| value.as_str().to_owned())
                .collect(),
            graph_count: parsed.graphs.len() as u64,
            node_count: parsed
                .graphs
                .iter()
                .map(|graph| graph.nodes.len() as u64)
                .sum(),
        })
    }

    fn diff(
        &self,
        old_definition: &Value,
        new_definition: &Value,
    ) -> Result<DiffResult, ManagementError> {
        let old = parse_definition(old_definition)?;
        let new = parse_definition(new_definition)?;
        let change = semantic_diff(&old, &new).map_err(|error| {
            ManagementError::invalid("canonicalization_failed", error.to_string())
        })?;
        Ok(DiffResult {
            old_definition_digest: raw_digest(old_definition)?,
            new_definition_digest: raw_digest(new_definition)?,
            semantic_change: change.executable_changed,
            changed_paths: if change.executable_changed {
                vec!["/".to_owned()]
            } else if change.annotations_changed {
                vec!["/annotations".to_owned()]
            } else {
                Vec::new()
            },
        })
    }
}

pub(super) fn parse_definition(value: &Value) -> Result<WorkflowDefinition, ManagementError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| ManagementError::invalid("definition_encode", error.to_string()))?;
    let definition: WorkflowDefinition = decode_strict(&bytes).map_err(decode_management_error)?;
    validate_definition(&definition).map_err(|error| {
        ManagementError::invalid(
            "definition_invalid",
            format!("workflow definition validation failed: {error}"),
        )
    })?;
    Ok(definition)
}

fn decode_management_error(error: DecodeError) -> ManagementError {
    ManagementError::invalid("definition_decode", error.to_string())
}

pub(super) fn raw_digest(value: &Value) -> Result<String, ManagementError> {
    super::contract::digest_value(value).map_err(ManagementError::from)
}

fn capability_diagnostics(
    definition: &WorkflowDefinition,
    manifest: &Value,
) -> Result<Vec<Diagnostic>, ManagementError> {
    let available = manifest
        .get("capabilities")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ManagementError::invalid(
                "capability_manifest",
                "capability manifest must contain a capabilities array",
            )
        })?
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    Ok(definition
        .capabilities
        .required
        .iter()
        .filter(|required| !available.contains(required.as_str()))
        .map(|required| Diagnostic {
            code: "capability_unavailable".to_owned(),
            severity: DiagnosticSeverity::Error,
            path: "$.capabilities.required".to_owned(),
            message: format!("required capability {} is unavailable", required.as_str()),
        })
        .collect())
}

/// Validates context references only when the owner has disclosed its bounded
/// binding catalog. An absent catalog preserves compatibility with owners that
/// have not yet installed the integration adapter; a malformed disclosed
/// catalog fails closed rather than silently accepting an unresolvable ref.
fn context_reference_diagnostics(
    definition: &WorkflowDefinition,
    manifest: &Value,
) -> Result<Vec<Diagnostic>, ManagementError> {
    let Some(entries) = manifest.get("context_bindings") else {
        return Ok(Vec::new());
    };
    let entries = entries.as_array().ok_or_else(|| {
        ManagementError::invalid(
            "context_binding_manifest",
            "context_bindings must be an array when disclosed",
        )
    })?;
    let mut bindings = BTreeMap::<String, BTreeSet<String>>::new();
    for entry in entries {
        let object = entry.as_object().ok_or_else(|| {
            ManagementError::invalid(
                "context_binding_manifest",
                "each context binding must be an object",
            )
        })?;
        if object.len() != 2 {
            return Err(ManagementError::invalid(
                "context_binding_manifest",
                "each context binding must contain context_ref and node_kinds only",
            ));
        }
        let context_ref = object
            .get("context_ref")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ManagementError::invalid(
                    "context_binding_manifest",
                    "each context binding needs a context_ref",
                )
            })?;
        validate_identifier("context_ref", context_ref)?;
        let kinds = object
            .get("node_kinds")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ManagementError::invalid(
                    "context_binding_manifest",
                    "each context binding needs node_kinds",
                )
            })?;
        if kinds.is_empty() {
            return Err(ManagementError::invalid(
                "context_binding_manifest",
                "each context binding needs at least one supported node kind",
            ));
        }
        let mut supported = BTreeSet::new();
        for kind in kinds {
            let kind = kind.as_str().ok_or_else(|| {
                ManagementError::invalid(
                    "context_binding_manifest",
                    "context binding node kinds must be strings",
                )
            })?;
            if !matches!(kind, "analyze" | "decide") {
                return Err(ManagementError::invalid(
                    "context_binding_manifest",
                    "context binding node kind is unsupported",
                ));
            }
            supported.insert(kind.to_owned());
        }
        if bindings.insert(context_ref.to_owned(), supported).is_some() {
            return Err(ManagementError::invalid(
                "context_binding_manifest",
                "context binding references must be unique",
            ));
        }
    }

    let mut diagnostics = Vec::new();
    for graph in &definition.graphs {
        for node in &graph.nodes {
            let (kind, context_ref) = match node {
                crate::workflow::NodeDefinition::Analyze { config, .. } => {
                    ("analyze", config.context_ref.as_str())
                }
                crate::workflow::NodeDefinition::Decide { config, .. } => {
                    ("decide", config.context_ref.as_str())
                }
                _ => continue,
            };
            let supported = bindings.get(context_ref);
            let code = if supported.is_none() {
                "context_ref_unresolved"
            } else if !supported.is_some_and(|kinds| kinds.contains(kind)) {
                "context_ref_incompatible"
            } else {
                continue;
            };
            diagnostics.push(Diagnostic {
                code: code.to_owned(),
                severity: DiagnosticSeverity::Error,
                path: format!(
                    "$.graphs.{}.nodes.{}.config.context_ref",
                    graph.id,
                    node.id()
                ),
                message: format!(
                    "context reference {context_ref} is not available for {kind} nodes"
                ),
            });
        }
    }
    Ok(diagnostics)
}
