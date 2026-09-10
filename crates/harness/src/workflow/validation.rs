// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, BTreeSet};

use super::definition::WorkflowDefinition;
use super::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticReport, StructuralLocation};
use super::graph_validation::{validate_graph, validate_graph_dependencies};
use super::ids::CapabilityId;

const MAX_GRAPHS: usize = 32;
const MAX_TOTAL_NODES: usize = 1024;
const MAX_TOTAL_EDGES: usize = 4096;
const MAX_CAPABILITIES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    report: DiagnosticReport,
}

impl ValidationError {
    pub fn diagnostics(&self) -> &[Diagnostic] {
        self.report.diagnostics()
    }

    pub fn contains(&self, code: DiagnosticCode) -> bool {
        self.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == code)
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "workflow validation failed with {} diagnostic(s)",
            self.diagnostics().len()
        )
    }
}

impl std::error::Error for ValidationError {}

pub fn validate_definition(definition: &WorkflowDefinition) -> Result<(), ValidationError> {
    let mut diagnostics = Vec::new();
    validate_capabilities(
        &definition.capabilities.required,
        &definition.capabilities.optional,
        &mut diagnostics,
    );
    validate_limits(definition, &mut diagnostics);
    if let Some(annotations) = &definition.annotations
        && (annotations.summary.is_empty() || annotations.summary.len() > 512)
    {
        push(
            &mut diagnostics,
            DiagnosticCode::InvalidLimit,
            "$.annotations.summary",
        );
    }
    if definition.graphs.is_empty() || definition.graphs.len() > MAX_GRAPHS {
        push(&mut diagnostics, DiagnosticCode::InvalidLimit, "$.graphs");
    }

    let mut graph_indexes = BTreeMap::new();
    let mut total_nodes = 0usize;
    let mut total_edges = 0usize;
    for (index, graph) in definition.graphs.iter().enumerate() {
        if graph_indexes.insert(graph.id.clone(), index).is_some() {
            push_at(
                &mut diagnostics,
                DiagnosticCode::DuplicateIdentifier,
                "$.graphs",
                index,
            );
        }
        total_nodes = total_nodes.saturating_add(graph.nodes.len());
        total_edges = total_edges.saturating_add(graph.edges.len());
    }
    if total_nodes > MAX_TOTAL_NODES {
        push(
            &mut diagnostics,
            DiagnosticCode::InvalidLimit,
            "$.graphs.nodes",
        );
    }
    if total_edges > MAX_TOTAL_EDGES {
        push(
            &mut diagnostics,
            DiagnosticCode::InvalidLimit,
            "$.graphs.edges",
        );
    }
    if !graph_indexes.contains_key(&definition.entry_graph) {
        push(
            &mut diagnostics,
            DiagnosticCode::MissingReference,
            "$.entry_graph",
        );
    }
    for (graph_index, graph) in definition.graphs.iter().enumerate() {
        validate_graph(
            definition.mode,
            graph,
            &graph_indexes,
            graph_index,
            &mut diagnostics,
        );
    }
    validate_graph_dependencies(&definition.graphs, &graph_indexes, &mut diagnostics);

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(ValidationError {
            report: DiagnosticReport::new(diagnostics),
        })
    }
}

fn validate_capabilities(
    required: &[CapabilityId],
    optional: &[CapabilityId],
    diagnostics: &mut Vec<Diagnostic>,
) {
    if required.len() > MAX_CAPABILITIES || optional.len() > MAX_CAPABILITIES {
        push(diagnostics, DiagnosticCode::InvalidLimit, "$.capabilities");
    }
    let mut seen = BTreeSet::new();
    for value in required {
        if !seen.insert(value) {
            push(
                diagnostics,
                DiagnosticCode::DuplicateIdentifier,
                "$.capabilities.required",
            );
        }
    }
    for value in optional {
        if !seen.insert(value) {
            push(
                diagnostics,
                DiagnosticCode::DuplicateIdentifier,
                "$.capabilities.optional",
            );
        }
    }
}

fn validate_limits(definition: &WorkflowDefinition, diagnostics: &mut Vec<Diagnostic>) {
    let limits = &definition.limits;
    if !(1..=4096).contains(&limits.max_steps)
        || !(1..=8).contains(&limits.max_subworkflow_depth)
        || limits.max_provider_calls > 4096
        || !(1..=4).contains(&limits.max_parallel_analyses)
        || limits.max_output_tokens > 2_000_000
    {
        push(diagnostics, DiagnosticCode::InvalidLimit, "$.limits");
    }
}

pub(super) fn push(diagnostics: &mut Vec<Diagnostic>, code: DiagnosticCode, path: &str) {
    diagnostics.push(Diagnostic::error(code, StructuralLocation::field(path)));
}

pub(super) fn push_at(
    diagnostics: &mut Vec<Diagnostic>,
    code: DiagnosticCode,
    path: &str,
    index: usize,
) {
    diagnostics.push(Diagnostic::error(
        code,
        StructuralLocation::field(format!("{path}[{index}]")),
    ));
}
