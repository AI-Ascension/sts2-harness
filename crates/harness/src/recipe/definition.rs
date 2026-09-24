// SPDX-License-Identifier: MIT

//! The authored recipe shape and the bounds applied at admission.

use std::collections::BTreeMap;

use super::ids::{RecipeId, RecipeRevision, StepId, ToolId, ToolRevision};

/// Largest number of steps one recipe may declare.
pub const MAX_STEPS: usize = 32;
/// Largest number of arguments one step may declare.
pub const MAX_ARGUMENTS_PER_STEP: usize = 16;
/// Largest number of dependencies one step may declare.
pub const MAX_DEPENDENCIES_PER_STEP: usize = 8;
/// Largest number of output slots one step may declare.
pub const MAX_OUTPUTS_PER_STEP: usize = 8;
/// Largest total argument payload, in bytes, for one step.
pub const MAX_ARGUMENT_BYTES: usize = 512;
/// Largest accepted per-step timeout, in milliseconds.
pub const MAX_TIMEOUT_MS: u32 = 60_000;
/// Largest accepted freshness horizon, in milliseconds.
pub const MAX_FRESHNESS_MS: u32 = 3_600_000;

/// Admission bounds; [`RecipeLimits::standard`] is the documented default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecipeLimits {
    /// Largest number of steps.
    pub max_steps: usize,
    /// Largest number of arguments per step.
    pub max_arguments_per_step: usize,
    /// Largest number of dependencies per step.
    pub max_dependencies_per_step: usize,
    /// Largest number of output slots per step.
    pub max_outputs_per_step: usize,
    /// Largest total argument payload, in bytes, per step.
    pub max_argument_bytes: usize,
    /// Largest per-step timeout, in milliseconds.
    pub max_timeout_ms: u32,
    /// Largest freshness horizon, in milliseconds.
    pub max_freshness_ms: u32,
}

impl RecipeLimits {
    /// The standard bounds every accepted recipe is measured against.
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            max_steps: MAX_STEPS,
            max_arguments_per_step: MAX_ARGUMENTS_PER_STEP,
            max_dependencies_per_step: MAX_DEPENDENCIES_PER_STEP,
            max_outputs_per_step: MAX_OUTPUTS_PER_STEP,
            max_argument_bytes: MAX_ARGUMENT_BYTES,
            max_timeout_ms: MAX_TIMEOUT_MS,
            max_freshness_ms: MAX_FRESHNESS_MS,
        }
    }
}

impl Default for RecipeLimits {
    fn default() -> Self {
        Self::standard()
    }
}

/// Whether a missing output blocks dispatch or records explicit absence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepRequirement {
    /// A missing or invalid result blocks provider dispatch.
    Required,
    /// Absence is recorded with explicit provenance and does not block.
    Optional,
}

/// One declared output of a step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputSlot {
    /// Output name within the step.
    pub name: String,
    /// Whether the output must be present for a required step.
    pub required: bool,
}

impl OutputSlot {
    /// A required output slot.
    #[must_use]
    pub fn required(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            required: true,
        }
    }

    /// An optional output slot whose absence is recorded explicitly.
    #[must_use]
    pub fn optional(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            required: false,
        }
    }
}

/// One ordered, typed read within a recipe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipeStep {
    /// Step identifier, unique within the recipe.
    pub id: StepId,
    /// Approved tool to invoke.
    pub tool: ToolId,
    /// Exact approved tool revision.
    pub tool_revision: ToolRevision,
    /// Argument schema this step declares; must match the catalog entry.
    pub argument_schema: String,
    /// Typed arguments drawn from admitted constants or prior outputs.
    pub arguments: BTreeMap<String, String>,
    /// Steps that must complete before this one.
    pub depends_on: Vec<StepId>,
    /// Declared outputs of this step.
    pub outputs: Vec<OutputSlot>,
    /// Whether a failed read blocks dispatch.
    pub requirement: StepRequirement,
    /// Per-step timeout in milliseconds.
    pub timeout_ms: u32,
    /// Freshness horizon in milliseconds; zero means never cached.
    pub freshness_ms: u32,
}

impl RecipeStep {
    /// Build a step with no arguments, dependencies or outputs yet declared.
    #[must_use]
    pub fn new(
        id: StepId,
        tool: ToolId,
        tool_revision: ToolRevision,
        requirement: StepRequirement,
    ) -> Self {
        Self {
            id,
            tool,
            tool_revision,
            argument_schema: String::new(),
            arguments: BTreeMap::new(),
            depends_on: Vec::new(),
            outputs: Vec::new(),
            requirement,
            timeout_ms: 1_000,
            freshness_ms: 0,
        }
    }
}

/// A whole authored recipe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipeDefinition {
    /// Recipe identifier.
    pub id: RecipeId,
    /// Recipe revision; zero is refused.
    pub revision: RecipeRevision,
    /// Identifier of the recipe-level output schema.
    pub output_schema: String,
    /// Steps in declared execution order.
    pub steps: Vec<RecipeStep>,
}

impl RecipeDefinition {
    /// Build a recipe with no steps yet declared.
    #[must_use]
    pub fn new(id: RecipeId, revision: RecipeRevision, output_schema: impl Into<String>) -> Self {
        Self {
            id,
            revision,
            output_schema: output_schema.into(),
            steps: Vec::new(),
        }
    }
}
