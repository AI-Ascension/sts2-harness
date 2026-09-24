// SPDX-License-Identifier: MIT

//! Bounded, versioned pre-agent read-only recipes and their admission.
//!
//! A recipe is an authored, deterministic list of approved read-only tool reads
//! that the harness performs *before* provider dispatch, so an admitted
//! workflow gathers game state and legal actions without a model tool call.
//! This module owns only the contract and its effect-free admission: it maps no
//! tool, starts no process and spends no provider call. Collection execution,
//! durable provenance and the Studio round-trip are separate slices of
//! [#97](https://github.com/AI-Ascension/sts2-harness/issues/97).
//!
//! Admission is total and ordered: [`admit_recipe`] measures a definition
//! against a fixed catalog and limits and reports the first violated property,
//! so a recipe with any prohibited step is refused whole rather than partially
//! executed.

mod admission;
mod catalog;
mod definition;
mod error;
mod ids;

pub use admission::{AdmittedRecipe, admit_recipe};
pub use catalog::{ApprovedTool, ReadOnlyToolCatalog, ToolClass};
pub use definition::{
    MAX_ARGUMENT_BYTES, MAX_ARGUMENTS_PER_STEP, MAX_DEPENDENCIES_PER_STEP, MAX_FRESHNESS_MS,
    MAX_OUTPUTS_PER_STEP, MAX_STEPS, MAX_TIMEOUT_MS, OutputSlot, RecipeDefinition, RecipeLimits,
    RecipeStep, StepRequirement,
};
pub use error::RecipeAdmissionError;
pub use ids::{
    MAX_IDENTIFIER_LEN, RecipeId, RecipeRevision, StepId, ToolId, ToolRevision, is_identifier,
};
