// SPDX-License-Identifier: MIT

//! The fixed set of tools a recipe may name, and their pinned revisions.

use super::ids::{ToolId, ToolRevision};

/// The harness-owned classification of a tool, not a description's claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolClass {
    /// The tool only reads state or reference data.
    ReadOnly,
    /// The tool can mutate game, profile or provider state.
    Mutation,
}

/// One approved tool pinned to exactly one revision and argument schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovedTool {
    /// Approved tool identifier.
    pub id: ToolId,
    /// Exact approved revision of this tool.
    pub revision: ToolRevision,
    /// Harness-owned classification; a read-only claim is not enough.
    pub class: ToolClass,
    /// Identifier of the argument schema a step must declare.
    pub argument_schema: String,
    /// Largest accepted result the mapping may return.
    pub max_result_bytes: u32,
}

impl ApprovedTool {
    /// Build an approved tool entry.
    #[must_use]
    pub fn new(
        id: ToolId,
        revision: ToolRevision,
        class: ToolClass,
        argument_schema: impl Into<String>,
        max_result_bytes: u32,
    ) -> Self {
        Self {
            id,
            revision,
            class,
            argument_schema: argument_schema.into(),
            max_result_bytes,
        }
    }

    /// Whether this entry is admitted for deterministic context gathering.
    #[must_use]
    pub fn is_read_only(&self) -> bool {
        self.class == ToolClass::ReadOnly
    }
}

/// A catalog of approved tools.
#[derive(Clone, Debug, Default)]
pub struct ReadOnlyToolCatalog {
    tools: Vec<ApprovedTool>,
}

impl ReadOnlyToolCatalog {
    /// Build a catalog from approved entries, preserving their order.
    #[must_use]
    pub fn new(tools: Vec<ApprovedTool>) -> Self {
        Self { tools }
    }

    /// The approved tool at exactly `id` and `revision`, when present.
    #[must_use]
    pub fn get(&self, id: &ToolId, revision: ToolRevision) -> Option<&ApprovedTool> {
        self.tools
            .iter()
            .find(|tool| tool.id == *id && tool.revision == revision)
    }

    /// Number of approved entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether no tool is approved.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}
