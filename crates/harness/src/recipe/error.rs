// SPDX-License-Identifier: MIT

//! Refusals returned by pre-agent recipe admission.

use std::fmt::{Display, Formatter};

/// The first property that made a recipe inadmissible.
///
/// Variants carry only structural identity, never a supplied argument value or
/// game text, so a refusal can be logged without leaking authored content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecipeAdmissionError {
    /// A revision was zero where a positive revision is required.
    ZeroRevision {
        /// Which revision was zero.
        field: &'static str,
    },
    /// A recipe-level or step-level schema identifier was not portable.
    InvalidSchema {
        /// The step whose argument schema is invalid, or `None` at recipe level.
        step: Option<String>,
    },
    /// The recipe declares no steps.
    EmptyRecipe,
    /// The recipe declares more steps than the bound permits.
    TooManySteps {
        /// Maximum accepted steps.
        bound: usize,
    },
    /// Two steps share an identifier.
    DuplicateStep {
        /// The repeated step identifier.
        step: String,
    },
    /// The named tool is not approved at the exact revision.
    UnknownTool {
        /// The step naming the tool.
        step: String,
        /// The unapproved tool identifier.
        tool: String,
        /// The unapproved revision.
        revision: u32,
    },
    /// The named tool can mutate state and must never gather context.
    MutationTool {
        /// The step naming the tool.
        step: String,
        /// The mutating tool identifier.
        tool: String,
    },
    /// The step's declared argument schema differs from the approved one.
    ArgumentSchemaMismatch {
        /// The step whose schema disagrees.
        step: String,
    },
    /// The step declares more arguments than the bound permits.
    TooManyArguments {
        /// The step declaring too many arguments.
        step: String,
        /// Maximum accepted arguments.
        bound: usize,
    },
    /// The step's argument payload exceeds the byte bound.
    ArgumentTooLarge {
        /// The step with the oversized payload.
        step: String,
        /// Maximum accepted bytes.
        bound: usize,
    },
    /// An argument name was not a portable identifier.
    InvalidArgument {
        /// The step with the invalid argument name.
        step: String,
    },
    /// The step declares more dependencies than the bound permits.
    TooManyDependencies {
        /// The step declaring too many dependencies.
        step: String,
        /// Maximum accepted dependencies.
        bound: usize,
    },
    /// A dependency names no step in this recipe.
    UnknownDependency {
        /// The step declaring the dependency.
        step: String,
        /// The unknown dependency.
        dependency: String,
    },
    /// A dependency names a step that does not run first.
    DependencyNotEarlier {
        /// The step declaring the dependency.
        step: String,
        /// The dependency that must run earlier.
        dependency: String,
    },
    /// A required step depends on an optional one, so a waiver could unblock it.
    RequiredDependsOnOptional {
        /// The required step.
        step: String,
        /// The optional dependency.
        dependency: String,
    },
    /// The step declares more output slots than the bound permits.
    TooManyOutputs {
        /// The step declaring too many outputs.
        step: String,
        /// Maximum accepted outputs.
        bound: usize,
    },
    /// An output name was not a portable identifier.
    InvalidOutput {
        /// The step with the invalid output name.
        step: String,
        /// The invalid output name.
        output: String,
    },
    /// Two output slots of one step share a name.
    DuplicateOutput {
        /// The step declaring the duplicate.
        step: String,
        /// The repeated output name.
        output: String,
    },
    /// The per-step timeout is zero or above the bound.
    TimeoutOutOfRange {
        /// The step with the out-of-range timeout.
        step: String,
        /// Maximum accepted timeout in milliseconds.
        bound: u32,
    },
    /// The freshness horizon exceeds the bound.
    FreshnessOutOfRange {
        /// The step with the out-of-range horizon.
        step: String,
        /// Maximum accepted freshness in milliseconds.
        bound: u32,
    },
}

impl Display for RecipeAdmissionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroRevision { field } => {
                write!(formatter, "recipe {field} revision must be positive")
            }
            Self::InvalidSchema { step: None } => {
                formatter.write_str("recipe output schema is not a portable identifier")
            }
            Self::InvalidSchema { step: Some(step) } => {
                write!(
                    formatter,
                    "recipe step {step} argument schema is not portable"
                )
            }
            Self::EmptyRecipe => formatter.write_str("recipe declares no steps"),
            Self::TooManySteps { bound } => {
                write!(formatter, "recipe exceeds the bound of {bound} steps")
            }
            Self::DuplicateStep { step } => {
                write!(formatter, "recipe step {step} is declared twice")
            }
            Self::UnknownTool {
                step,
                tool,
                revision,
            } => write!(
                formatter,
                "recipe step {step} names unapproved tool {tool} at revision {revision}"
            ),
            Self::MutationTool { step, tool } => {
                write!(formatter, "recipe step {step} names mutating tool {tool}")
            }
            Self::ArgumentSchemaMismatch { step } => write!(
                formatter,
                "recipe step {step} declares an argument schema the tool revision does not own"
            ),
            Self::TooManyArguments { step, bound } => write!(
                formatter,
                "recipe step {step} exceeds the bound of {bound} arguments"
            ),
            Self::ArgumentTooLarge { step, bound } => write!(
                formatter,
                "recipe step {step} exceeds the bound of {bound} argument bytes"
            ),
            Self::InvalidArgument { step } => {
                write!(
                    formatter,
                    "recipe step {step} names a non-portable argument"
                )
            }
            Self::TooManyDependencies { step, bound } => write!(
                formatter,
                "recipe step {step} exceeds the bound of {bound} dependencies"
            ),
            Self::UnknownDependency { step, dependency } => write!(
                formatter,
                "recipe step {step} depends on unknown step {dependency}"
            ),
            Self::DependencyNotEarlier { step, dependency } => write!(
                formatter,
                "recipe step {step} must depend on an earlier step, not {dependency}"
            ),
            Self::RequiredDependsOnOptional { step, dependency } => write!(
                formatter,
                "required recipe step {step} depends on optional step {dependency}"
            ),
            Self::TooManyOutputs { step, bound } => write!(
                formatter,
                "recipe step {step} exceeds the bound of {bound} outputs"
            ),
            Self::InvalidOutput { step, output } => {
                write!(
                    formatter,
                    "recipe step {step} names non-portable output {output}"
                )
            }
            Self::DuplicateOutput { step, output } => {
                write!(
                    formatter,
                    "recipe step {step} declares output {output} twice"
                )
            }
            Self::TimeoutOutOfRange { step, bound } => write!(
                formatter,
                "recipe step {step} timeout must be 1..={bound} milliseconds"
            ),
            Self::FreshnessOutOfRange { step, bound } => write!(
                formatter,
                "recipe step {step} freshness exceeds {bound} milliseconds"
            ),
        }
    }
}

impl std::error::Error for RecipeAdmissionError {}
