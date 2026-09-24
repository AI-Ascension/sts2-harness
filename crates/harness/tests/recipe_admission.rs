// SPDX-License-Identifier: MIT

#![allow(clippy::unwrap_used, clippy::panic)]

//! Admission matrix for the pre-agent read-only recipe contract (issue #97, T1).

use sts2_harness::recipe::{
    ApprovedTool, OutputSlot, ReadOnlyToolCatalog, RecipeAdmissionError, RecipeDefinition,
    RecipeId, RecipeLimits, RecipeRevision, RecipeStep, StepId, StepRequirement, ToolClass, ToolId,
    ToolRevision, admit_recipe,
};

const OBSERVE_ARGS: &str = "sts2.observe.args.v1";
const LEGAL_ARGS: &str = "sts2.legal_actions.args.v1";
const MUTATE_ARGS: &str = "sts2.play_card.args.v1";
const RESULT_SCHEMA: &str = "sts2.recipe.result.v1";

fn tool(id: &str) -> ToolId {
    ToolId::new(id).unwrap()
}

fn step(id: &str) -> StepId {
    StepId::new(id).unwrap()
}

fn recipe(id: &str) -> RecipeId {
    RecipeId::new(id).unwrap()
}

fn catalog() -> ReadOnlyToolCatalog {
    ReadOnlyToolCatalog::new(vec![
        ApprovedTool::new(
            tool("sts2.observe"),
            ToolRevision::new(1),
            ToolClass::ReadOnly,
            OBSERVE_ARGS,
            65_536,
        ),
        ApprovedTool::new(
            tool("sts2.legal_actions"),
            ToolRevision::new(1),
            ToolClass::ReadOnly,
            LEGAL_ARGS,
            65_536,
        ),
        ApprovedTool::new(
            tool("sts2.play_card"),
            ToolRevision::new(3),
            ToolClass::Mutation,
            MUTATE_ARGS,
            4_096,
        ),
    ])
}

fn observe_step(requirement: StepRequirement) -> RecipeStep {
    let mut observe = RecipeStep::new(
        step("observe"),
        tool("sts2.observe"),
        ToolRevision::new(1),
        requirement,
    );
    observe.argument_schema = OBSERVE_ARGS.to_owned();
    observe.outputs = vec![
        OutputSlot::required("state"),
        OutputSlot::optional("actions"),
    ];
    observe
}

fn legal_step(depends_on: &[&str], requirement: StepRequirement) -> RecipeStep {
    let mut legal = RecipeStep::new(
        step("legal"),
        tool("sts2.legal_actions"),
        ToolRevision::new(1),
        requirement,
    );
    legal.argument_schema = LEGAL_ARGS.to_owned();
    legal.depends_on = depends_on.iter().map(|id| step(id)).collect();
    legal.outputs = vec![OutputSlot::required("legal_actions")];
    legal
}

fn base_recipe() -> RecipeDefinition {
    let mut definition = RecipeDefinition::new(
        recipe("sts2.baseline"),
        RecipeRevision::new(1),
        RESULT_SCHEMA,
    );
    definition.steps = vec![
        observe_step(StepRequirement::Required),
        legal_step(&["observe"], StepRequirement::Required),
    ];
    definition
}

fn admit(definition: &RecipeDefinition) -> Result<(), RecipeAdmissionError> {
    admit_recipe(definition, &catalog(), &RecipeLimits::standard()).map(|_| ())
}

#[test]
fn admits_a_valid_recipe_in_declared_order() {
    let definition = base_recipe();
    let admitted = admit_recipe(&definition, &catalog(), &RecipeLimits::standard()).unwrap();
    let ids: Vec<&str> = admitted
        .ordered_steps()
        .iter()
        .map(|step| step.id.as_str())
        .collect();
    assert_eq!(ids, ["observe", "legal"]);
    assert_eq!(admitted.definition(), &definition);
}

#[test]
fn admission_is_deterministic() {
    let definition = base_recipe();
    let first = admit_recipe(&definition, &catalog(), &RecipeLimits::standard()).unwrap();
    let second = admit_recipe(&definition, &catalog(), &RecipeLimits::standard()).unwrap();
    assert_eq!(first.ordered_steps(), second.ordered_steps());
    assert_eq!(first.definition(), second.definition());
}

#[test]
fn refuses_a_zero_recipe_revision() {
    let mut definition = base_recipe();
    definition.revision = RecipeRevision::new(0);
    assert_eq!(
        admit(&definition),
        Err(RecipeAdmissionError::ZeroRevision { field: "recipe" })
    );
}

#[test]
fn refuses_a_non_portable_result_schema() {
    let mut definition = base_recipe();
    definition.output_schema = "sts2 result/v1".to_owned();
    assert_eq!(
        admit(&definition),
        Err(RecipeAdmissionError::InvalidSchema { step: None })
    );
}

#[test]
fn refuses_an_empty_recipe() {
    let mut definition = base_recipe();
    definition.steps.clear();
    assert_eq!(admit(&definition), Err(RecipeAdmissionError::EmptyRecipe));
}

#[test]
fn refuses_more_steps_than_the_bound() {
    let mut definition = base_recipe();
    let bound = RecipeLimits::standard().max_steps;
    definition.steps = (0..=bound)
        .map(|index| {
            let mut observe = observe_step(StepRequirement::Required);
            observe.id = step(&format!("observe_{index}"));
            observe
        })
        .collect();
    assert_eq!(
        admit(&definition),
        Err(RecipeAdmissionError::TooManySteps { bound })
    );
}

#[test]
fn refuses_a_duplicate_step_identifier() {
    let mut definition = base_recipe();
    definition
        .steps
        .push(observe_step(StepRequirement::Required));
    assert_eq!(
        admit(&definition),
        Err(RecipeAdmissionError::DuplicateStep {
            step: "observe".to_owned()
        })
    );
}

#[test]
fn refuses_an_unapproved_tool_revision() {
    let mut definition = base_recipe();
    definition.steps[0].tool_revision = ToolRevision::new(2);
    assert_eq!(
        admit(&definition),
        Err(RecipeAdmissionError::UnknownTool {
            step: "observe".to_owned(),
            tool: "sts2.observe".to_owned(),
            revision: 2,
        })
    );
}

#[test]
fn refuses_a_mutating_tool_classified_as_a_read() {
    let mut definition = base_recipe();
    let mut mutation = RecipeStep::new(
        step("play"),
        tool("sts2.play_card"),
        ToolRevision::new(3),
        StepRequirement::Required,
    );
    mutation.argument_schema = MUTATE_ARGS.to_owned();
    definition.steps = vec![mutation];
    assert_eq!(
        admit(&definition),
        Err(RecipeAdmissionError::MutationTool {
            step: "play".to_owned(),
            tool: "sts2.play_card".to_owned(),
        })
    );
}

#[test]
fn refuses_a_mismatched_argument_schema() {
    let mut definition = base_recipe();
    definition.steps[0].argument_schema = "sts2.observe.args.v2".to_owned();
    assert_eq!(
        admit(&definition),
        Err(RecipeAdmissionError::ArgumentSchemaMismatch {
            step: "observe".to_owned()
        })
    );
}

#[test]
fn refuses_a_non_portable_argument_name() {
    let mut definition = base_recipe();
    definition.steps[0]
        .arguments
        .insert("bad key".to_owned(), "1".to_owned());
    assert_eq!(
        admit(&definition),
        Err(RecipeAdmissionError::InvalidArgument {
            step: "observe".to_owned()
        })
    );
}

#[test]
fn refuses_too_many_arguments() {
    let mut definition = base_recipe();
    let bound = RecipeLimits::standard().max_arguments_per_step;
    for index in 0..=bound {
        definition.steps[0]
            .arguments
            .insert(format!("arg{index}"), "1".to_owned());
    }
    assert_eq!(
        admit(&definition),
        Err(RecipeAdmissionError::TooManyArguments {
            step: "observe".to_owned(),
            bound
        })
    );
}

#[test]
fn refuses_an_oversized_argument_payload() {
    let mut definition = base_recipe();
    let bound = RecipeLimits::standard().max_argument_bytes;
    definition.steps[0]
        .arguments
        .insert("seed".to_owned(), "x".repeat(bound));
    assert_eq!(
        admit(&definition),
        Err(RecipeAdmissionError::ArgumentTooLarge {
            step: "observe".to_owned(),
            bound
        })
    );
}

#[test]
fn refuses_an_unknown_dependency() {
    let mut definition = base_recipe();
    definition.steps[1].depends_on = vec![step("missing")];
    assert_eq!(
        admit(&definition),
        Err(RecipeAdmissionError::UnknownDependency {
            step: "legal".to_owned(),
            dependency: "missing".to_owned(),
        })
    );
}

#[test]
fn refuses_a_dependency_that_does_not_run_first() {
    let mut definition = base_recipe();
    definition.steps[0].depends_on = vec![step("legal")];
    assert_eq!(
        admit(&definition),
        Err(RecipeAdmissionError::DependencyNotEarlier {
            step: "observe".to_owned(),
            dependency: "legal".to_owned(),
        })
    );
}

#[test]
fn refuses_a_required_step_that_depends_on_an_optional_step() {
    let mut definition = base_recipe();
    definition.steps[0].requirement = StepRequirement::Optional;
    assert_eq!(
        admit(&definition),
        Err(RecipeAdmissionError::RequiredDependsOnOptional {
            step: "legal".to_owned(),
            dependency: "observe".to_owned(),
        })
    );
}

#[test]
fn refuses_a_duplicate_output_name() {
    let mut definition = base_recipe();
    definition.steps[0]
        .outputs
        .push(OutputSlot::optional("state"));
    assert_eq!(
        admit(&definition),
        Err(RecipeAdmissionError::DuplicateOutput {
            step: "observe".to_owned(),
            output: "state".to_owned(),
        })
    );
}

#[test]
fn refuses_a_zero_or_oversized_timeout() {
    let mut definition = base_recipe();
    definition.steps[0].timeout_ms = 0;
    let bound = RecipeLimits::standard().max_timeout_ms;
    assert_eq!(
        admit(&definition),
        Err(RecipeAdmissionError::TimeoutOutOfRange {
            step: "observe".to_owned(),
            bound
        })
    );

    let mut definition = base_recipe();
    definition.steps[0].timeout_ms = bound + 1;
    assert_eq!(
        admit(&definition),
        Err(RecipeAdmissionError::TimeoutOutOfRange {
            step: "observe".to_owned(),
            bound
        })
    );
}

#[test]
fn refuses_a_freshness_horizon_beyond_the_bound() {
    let mut definition = base_recipe();
    let bound = RecipeLimits::standard().max_freshness_ms;
    definition.steps[0].freshness_ms = bound + 1;
    assert_eq!(
        admit(&definition),
        Err(RecipeAdmissionError::FreshnessOutOfRange {
            step: "observe".to_owned(),
            bound
        })
    );
}

#[test]
fn ids_reject_non_portable_text() {
    assert!(ToolId::new("sts2.observe").is_some());
    assert!(ToolId::new("").is_none());
    assert!(ToolId::new("sts2 observe").is_none());
    assert!(ToolId::new("x".repeat(200)).is_none());
}
