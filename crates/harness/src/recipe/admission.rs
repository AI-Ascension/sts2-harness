// SPDX-License-Identifier: MIT

//! Effect-free admission of a pre-agent recipe.

use std::collections::BTreeSet;

use super::catalog::{ReadOnlyToolCatalog, ToolClass};
use super::definition::{RecipeDefinition, RecipeLimits, RecipeStep, StepRequirement};
use super::error::RecipeAdmissionError;
use super::ids::is_identifier;

/// A recipe that passed every admission check, in declared order.
#[derive(Clone, Debug)]
pub struct AdmittedRecipe {
    definition: RecipeDefinition,
}

impl AdmittedRecipe {
    /// The admitted definition.
    #[must_use]
    pub fn definition(&self) -> &RecipeDefinition {
        &self.definition
    }

    /// Steps in execution order.
    ///
    /// A dependency must name an earlier step, so declared order is already a
    /// valid topological order and admission never reorders the recipe.
    #[must_use]
    pub fn ordered_steps(&self) -> &[RecipeStep] {
        &self.definition.steps
    }
}

/// Admit `definition` against a fixed catalog and limits, or refuse the first
/// violated property.
///
/// Admission performs no read and reserves no inference; it only proves the
/// recipe names approved read-only tools in a decidable order.
pub fn admit_recipe(
    definition: &RecipeDefinition,
    catalog: &ReadOnlyToolCatalog,
    limits: &RecipeLimits,
) -> Result<AdmittedRecipe, RecipeAdmissionError> {
    if definition.revision.get() == 0 {
        return Err(RecipeAdmissionError::ZeroRevision { field: "recipe" });
    }
    if !is_identifier(&definition.output_schema) {
        return Err(RecipeAdmissionError::InvalidSchema { step: None });
    }
    if definition.steps.is_empty() {
        return Err(RecipeAdmissionError::EmptyRecipe);
    }
    if definition.steps.len() > limits.max_steps {
        return Err(RecipeAdmissionError::TooManySteps {
            bound: limits.max_steps,
        });
    }

    let mut seen = BTreeSet::new();
    for (index, step) in definition.steps.iter().enumerate() {
        if !seen.insert(step.id.as_str()) {
            return Err(RecipeAdmissionError::DuplicateStep {
                step: step.id.as_str().to_owned(),
            });
        }
        admit_step(index, step, definition, catalog, limits)?;
    }

    Ok(AdmittedRecipe {
        definition: definition.clone(),
    })
}

fn admit_step(
    index: usize,
    step: &RecipeStep,
    definition: &RecipeDefinition,
    catalog: &ReadOnlyToolCatalog,
    limits: &RecipeLimits,
) -> Result<(), RecipeAdmissionError> {
    let name = step.id.as_str().to_owned();
    if step.tool_revision.get() == 0 {
        return Err(RecipeAdmissionError::ZeroRevision { field: "tool" });
    }
    let tool = catalog.get(&step.tool, step.tool_revision).ok_or_else(|| {
        RecipeAdmissionError::UnknownTool {
            step: name.clone(),
            tool: step.tool.as_str().to_owned(),
            revision: step.tool_revision.get(),
        }
    })?;
    if tool.class != ToolClass::ReadOnly {
        return Err(RecipeAdmissionError::MutationTool {
            step: name,
            tool: step.tool.as_str().to_owned(),
        });
    }
    if step.argument_schema != tool.argument_schema || !is_identifier(&step.argument_schema) {
        return Err(RecipeAdmissionError::ArgumentSchemaMismatch { step: name });
    }
    admit_arguments(step, limits)?;
    admit_dependencies(index, step, definition, limits)?;
    admit_outputs(step, limits)?;
    admit_limits(step, limits)
}

fn admit_arguments(step: &RecipeStep, limits: &RecipeLimits) -> Result<(), RecipeAdmissionError> {
    let name = step.id.as_str().to_owned();
    if step.arguments.len() > limits.max_arguments_per_step {
        return Err(RecipeAdmissionError::TooManyArguments {
            step: name,
            bound: limits.max_arguments_per_step,
        });
    }
    let mut bytes = 0usize;
    for (argument, value) in &step.arguments {
        if !is_identifier(argument) {
            return Err(RecipeAdmissionError::InvalidArgument { step: name });
        }
        bytes = bytes
            .saturating_add(argument.len())
            .saturating_add(value.len());
    }
    if bytes > limits.max_argument_bytes {
        return Err(RecipeAdmissionError::ArgumentTooLarge {
            step: name,
            bound: limits.max_argument_bytes,
        });
    }
    Ok(())
}

fn admit_dependencies(
    index: usize,
    step: &RecipeStep,
    definition: &RecipeDefinition,
    limits: &RecipeLimits,
) -> Result<(), RecipeAdmissionError> {
    let name = step.id.as_str().to_owned();
    if step.depends_on.len() > limits.max_dependencies_per_step {
        return Err(RecipeAdmissionError::TooManyDependencies {
            step: name,
            bound: limits.max_dependencies_per_step,
        });
    }
    for dependency in &step.depends_on {
        let earlier = definition
            .steps
            .iter()
            .take(index)
            .position(|prior| prior.id == *dependency);
        let Some(position) = earlier else {
            let known = definition.steps.iter().any(|other| other.id == *dependency);
            return Err(if known {
                RecipeAdmissionError::DependencyNotEarlier {
                    step: name,
                    dependency: dependency.as_str().to_owned(),
                }
            } else {
                RecipeAdmissionError::UnknownDependency {
                    step: name,
                    dependency: dependency.as_str().to_owned(),
                }
            });
        };
        if step.requirement == StepRequirement::Required
            && definition.steps[position].requirement == StepRequirement::Optional
        {
            return Err(RecipeAdmissionError::RequiredDependsOnOptional {
                step: name,
                dependency: dependency.as_str().to_owned(),
            });
        }
    }
    Ok(())
}

fn admit_outputs(step: &RecipeStep, limits: &RecipeLimits) -> Result<(), RecipeAdmissionError> {
    let name = step.id.as_str().to_owned();
    if step.outputs.len() > limits.max_outputs_per_step {
        return Err(RecipeAdmissionError::TooManyOutputs {
            step: name,
            bound: limits.max_outputs_per_step,
        });
    }
    let mut names = BTreeSet::new();
    for (slot_index, slot) in step.outputs.iter().enumerate() {
        if !is_identifier(&slot.name) {
            return Err(RecipeAdmissionError::InvalidOutput {
                step: name,
                slot_index,
            });
        }
        if !names.insert(slot.name.as_str()) {
            return Err(RecipeAdmissionError::DuplicateOutput {
                step: name,
                output: slot.name.clone(),
            });
        }
    }
    Ok(())
}

fn admit_limits(step: &RecipeStep, limits: &RecipeLimits) -> Result<(), RecipeAdmissionError> {
    let name = step.id.as_str().to_owned();
    if step.timeout_ms == 0 || step.timeout_ms > limits.max_timeout_ms {
        return Err(RecipeAdmissionError::TimeoutOutOfRange {
            step: name,
            bound: limits.max_timeout_ms,
        });
    }
    if step.freshness_ms > limits.max_freshness_ms {
        return Err(RecipeAdmissionError::FreshnessOutOfRange {
            step: name,
            bound: limits.max_freshness_ms,
        });
    }
    Ok(())
}
