// SPDX-License-Identifier: MIT

//! Tests for the advisory plan: what it admits, and what ends it.
//!
//! `action_plan.rs` carried the whole plan contract with no tests beside it, so every property
//! below was previously asserted only by reading the code. These drive the real `ActionPlan`
//! against real `DecisionInput` values, so they fail if the logic moves, not just if a comment
//! changes.
//!
//! The load-bearing property is the negative one: a plan is a *prediction* about a state that has
//! not happened yet, so every way it can be wrong must end it rather than be coerced, substituted
//! or retried. A test that only proved the happy path would not distinguish a plan from a queue.

#![allow(clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};

use super::*;
use crate::episode::legal_actions::{ActionKind, EpisodeLegalAction, EpisodeLegalActionSet};
use crate::episode::observation::{EpisodeObservation, EpisodeStage};

/// The card the plan is written against. Playing it is what the plan predicts will happen.
fn strike() -> Value {
    json!({"card_id":"card-1","name":"Strike","cost":1,"upgraded":false})
}

/// Builds a decision input whose catalog and observation agree.
///
/// Every knob a test needs to *perturb* -- the offered catalogue, the player HP, the hand -- is a
/// parameter, because divergence is the thing under test: a plan must be able to notice that the
/// state moved underneath it, and a builder that hard-coded the interesting values could not
/// express that.
fn combat_input() -> DecisionInput {
    combat_with(
        1,
        "combat-1",
        vec!["play-card", "end-turn"],
        json!([strike()]),
        80,
    )
}

/// A follow-up state: the card is gone from the hand, so only `end-turn` remains offered.
fn advanced() -> DecisionInput {
    combat_with(2, "combat-2", vec!["end-turn"], json!([]), 80)
}

fn combat_with(
    generation: u64,
    state_id: &str,
    offered: Vec<&str>,
    hand: Value,
    hp: i64,
) -> DecisionInput {
    let catalog: Vec<Value> = offered
        .iter()
        .map(|id| {
            json!({
                "action_id": id,
                "action": match *id {
                    "play-card" => {
                        json!({"kind":"play_card","card_id":"card-1","target_id":"enemy-1"})
                    }
                    "end-turn" => json!({"kind":"end_turn"}),
                    other => panic!("unexpected test action {other}"),
                },
            })
        })
        .collect();
    let actions = offered
        .iter()
        .map(|id| {
            let kind = match *id {
                "play-card" => ActionKind::PlayCard,
                "end-turn" => ActionKind::EndTurn,
                other => panic!("unexpected test action {other}"),
            };
            EpisodeLegalAction::new(*id, kind).expect("test action identity")
        })
        .collect();
    let observation = EpisodeObservation::new(
        state_id,
        generation,
        EpisodeStage::Combat,
        true,
        false,
        true,
        json!({
            "state_id": state_id,
            "generation": generation,
            "visible_seed": "PLAN-SEED",
            "player": {
                "hp": hp, "max_hp": 80, "energy": 3, "gold": 99,
                "hand": hand,
                "deck": [], "discard": [], "exhaust": []
            },
            "state": {"state":"combat","turn_index":1,"enemies":[]},
            "legal_actions": catalog
        }),
    )
    .expect("plan observation");
    let action_set = EpisodeLegalActionSet::new(state_id, generation, actions).expect("action set");
    DecisionInput::new(
        ModelExecutionId::new(generation).expect("execution"),
        observation,
        action_set,
        "complete the combat",
        Vec::new(),
    )
}

/// A two-step plan over the generation-1 catalog: play the card, then end the turn.
fn plan_over(input: &DecisionInput) -> ActionPlan {
    ActionPlan::new(
        input,
        &["play-card".to_owned(), "end-turn".to_owned()],
        "play then end".to_owned(),
    )
    .expect("plan admitted")
}

#[test]
fn a_plan_expands_one_step_at_a_time_and_only_after_settlement() {
    let first = combat_input();
    let mut plan = plan_over(&first);
    let step = plan.next(&first, true).expect("first step");
    assert!(
        matches!(&step, Decision::Action { action_id, .. } if action_id == "play-card"),
        "the first step is the first action, got {step:?}"
    );
    // Before settlement the plan owes an outcome, and asking again is refused rather than
    // re-answering the same step against a state that has not moved.
    assert!(plan.awaiting_settlement());
    assert!(
        plan.next(&first, false).is_none(),
        "a plan awaiting settlement must not yield the same step twice"
    );
    plan.action_completed(true);
    assert!(!plan.awaiting_settlement());
}

#[test]
fn a_plan_step_the_host_no_longer_offers_ends_the_plan() {
    let first = combat_with(1, "combat-1", vec!["play-card"], json!([strike()]), 80);
    let mut plan = ActionPlan::new(&first, &["play-card".to_owned()], "single step".to_owned())
        .expect("plan admitted");
    assert!(plan.next(&first, true).is_some());
    plan.action_completed(true);

    // The card left the hand, so `play-card` is no longer offered. The host still offers
    // `end-turn`, but that is not the step this plan was on: re-deriving against the live catalog
    // must not substitute another action for a step the host withdrew.
    let second = advanced();
    assert!(
        plan.next(&second, false).is_none(),
        "a step the host no longer offers must end the plan, not fall through to another action"
    );
}

#[test]
fn a_plan_step_the_host_stops_offering_mid_plan_ends_the_plan() {
    let first = combat_input();
    let mut plan = plan_over(&first);
    assert!(plan.next(&first, true).is_some());
    plan.action_completed(true);

    // The strongest form of this contract: the host has not moved on at all. The same generation
    // still predicts the same state, and `play-card` is still offered -- but the plan is on its
    // `end-turn` step and the host has withdrawn it. Nothing here may excuse that, so re-deriving
    // the step against the live catalog is the only thing that can end the plan.
    let withdrawn = combat_with(1, "combat-1", vec!["play-card"], json!([]), 80);
    assert!(
        plan.next(&withdrawn, false).is_none(),
        "a step the host no longer offers must end the plan even when nothing else diverged"
    );
}

#[test]
fn divergence_in_the_state_the_step_assumed_ends_the_plan() {
    let first = combat_input();
    let mut plan = plan_over(&first);
    assert!(plan.next(&first, true).is_some());
    plan.action_completed(true);

    // The generation advanced and the catalogue still offers everything, but the observed state
    // no longer matches what the plan assumed: the player took damage. This is not the transition
    // the plan predicted, so nothing further may be dispatched from it.
    let damaged = combat_with(2, "combat-2", vec!["play-card", "end-turn"], json!([]), 12);
    assert!(
        plan.next(&damaged, false).is_none(),
        "a state the plan did not assume must end it"
    );
}

#[test]
fn a_generation_that_has_not_advanced_ends_the_plan() {
    let first = combat_input();
    let mut plan = plan_over(&first);
    assert!(plan.next(&first, true).is_some());
    plan.action_completed(true);

    // The plan requires the host to have moved on. Re-deciding the same generation would dispatch
    // a step against a state the host has not left. Nothing else may be what ends the plan here:
    // the catalogue still offers the step, and the hand is emptied exactly as the plan predicted,
    // so the generation counter is the only thing left to refuse on.
    assert!(
        plan.next(
            &combat_with(1, "combat-1", vec!["play-card", "end-turn"], json!([]), 80),
            false
        )
        .is_none(),
        "a plan step must not dispatch against a generation the host has not left"
    );
}

#[test]
fn an_unsettled_step_discards_the_remaining_plan() {
    let first = combat_input();
    let mut plan = plan_over(&first);
    assert!(plan.next(&first, true).is_some());
    // `settled = false` means the effect's outcome is unknown. Continuing to predict from an
    // unknown state is exactly the invention this contract refuses, so the rest is dropped.
    plan.action_completed(false);
    // The host left exactly the state the plan predicted -- the next step is still offered and the
    // hand is emptied as expected -- so the disposal of the remainder is the only thing that can
    // stop this dispatch. An unsettled step does not license a fresh prediction.
    assert!(
        plan.next(&advanced(), false).is_none(),
        "an unsettled step must discard the remainder"
    );
    // Recorded explicitly, because the guarantee is about the plan's own contents and not merely
    // about what the next dispatch happens to return.
    assert!(
        plan.actions.is_empty(),
        "an unsettled step must drop the remaining steps from the plan itself"
    );
}

#[test]
fn an_action_the_host_never_offered_is_refused_when_the_plan_is_built() {
    let first = combat_input();
    let error = ActionPlan::new(
        &first,
        &["play-card".to_owned(), "not-offered".to_owned()],
        "plan naming an unavailable action".to_owned(),
    )
    .err()
    .expect("an action outside the catalog must be refused");
    assert_eq!(error, PolicyError::IllegalAction);
}

#[test]
fn a_duplicated_or_empty_or_oversized_plan_is_refused() {
    let first = combat_input();
    // A repeated id is unanswerable: the same action cannot be two different steps.
    assert_eq!(
        ActionPlan::new(
            &first,
            &["play-card".to_owned(), "play-card".to_owned()],
            "duplicate".to_owned(),
        )
        .err()
        .expect("a duplicated step must be refused"),
        PolicyError::IllegalAction
    );
    // An empty plan is not a plan.
    assert_eq!(
        ActionPlan::new(&first, &[], "empty".to_owned())
            .err()
            .expect("an empty plan must be refused"),
        PolicyError::MalformedDecision
    );
    // And the step count stays bounded.
    let too_many: Vec<String> = (0..9).map(|index| format!("end-turn-{index}")).collect();
    assert_eq!(
        ActionPlan::new(&first, &too_many, "oversized".to_owned())
            .err()
            .expect("an oversized plan must be refused"),
        PolicyError::MalformedDecision
    );
}

#[test]
fn a_sequence_the_stage_does_not_permit_is_refused() {
    let first = combat_input();
    // Only a single step may be a terminal action. `end_turn` followed by another action is not a
    // sequence the host can walk, so the plan is refused at build time rather than discovering it
    // one dispatch later.
    let error = ActionPlan::new(
        &first,
        &["end-turn".to_owned(), "play-card".to_owned()],
        "terminal step followed by another action".to_owned(),
    )
    .err()
    .expect("a step after a terminal action must be refused");
    assert_eq!(error, PolicyError::MalformedDecision);
}
