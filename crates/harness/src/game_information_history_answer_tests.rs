// SPDX-License-Identifier: MIT

//! The answers the history tool gives, and the ones it refuses to give.
//!
//! Split out of `history_tool_tests`, which owns the fixtures these tests ask through.

use super::*;
use crate::semantic_history::{
    SemanticHistoryCoverage, SemanticHistoryCoverageInterval, SemanticHistoryCoverageStatus,
};
use serde_json::json;

/// One event whose coverage is a disclosed gap, carrying no observed detail at all.
fn gap_event(event_id: &str, sequence: u64) -> SemanticHistoryEventInput {
    SemanticHistoryEventInput {
        subjects: Vec::new(),
        reference: None,
        coverage: SemanticHistoryCoverage::gap(
            SemanticHistoryCoverageStatus::Dropped,
            "capture_dropped",
        ),
        ..event(event_id, SemanticHistoryKind::CardPlayed, sequence)
    }
}

#[test]
fn an_explanation_states_what_it_knows_and_what_it_does_not() -> TestResult {
    let (mut session, _corpus) = setup(8192)?;
    let mut store = open(SemanticHistoryCaptureWindow::complete(1))?;
    append(
        &mut store,
        "choice_1",
        SemanticHistoryKind::ChoiceMade,
        1,
        None,
    )?;
    append(
        &mut store,
        "choice_2",
        SemanticHistoryKind::ChoiceMade,
        2,
        Some("choice_1"),
    )?;
    append(
        &mut store,
        "choice_3",
        SemanticHistoryKind::ChoiceMade,
        3,
        Some("choice_2"),
    )?;
    session.attach_history(store)?;
    let mut bounded = explain(ROOT, "choice_3");
    bounded["limits"] = json!({"max_depth":1,"max_visits":4});
    let truncated = answered(ask(&session, bounded))?;
    assert_eq!(truncated["event_id"], json!("choice_3"));
    assert_eq!(truncated["kind"], json!("choice_made"));
    assert_eq!(truncated["authority"], json!(HISTORY_AUTHORITY));
    assert_eq!(truncated["traversal"]["root_event_id"], json!("choice_3"));
    assert_eq!(truncated["traversal"]["root_cause_unstated"], json!(false));
    assert_eq!(
        truncated["traversal"]["visited"],
        json!(["choice_3", "choice_2"]),
        "the walk names exactly the events it visited"
    );
    assert_eq!(
        truncated["traversal"]["truncated"],
        json!(true),
        "a walk stopped at its own bound says so"
    );
    assert_eq!(
        truncated["traversal"]["links"],
        json!([{"from_event_id":"choice_2","to_event_id":"choice_3","depth":1}])
    );
    // The default walk is the boundary's own bound and completes this chain.
    let full = answered(ask(&session, explain(ROOT, "choice_3")))?;
    assert_eq!(full["traversal"]["truncated"], json!(false));
    assert_eq!(full["traversal"]["links"].as_array().map(Vec::len), Some(2));
    // A root with no stated cause says so rather than implying one.
    let unstated = answered(ask(&session, explain(ROOT, "choice_1")))?;
    assert_eq!(unstated["traversal"]["root_cause_unstated"], json!(true));
    assert_eq!(unstated["traversal"]["links"], json!([]));
    assert_eq!(unstated["traversal"]["truncated"], json!(false));
    for (branch, event_id, expected) in [
        (ROOT, "choice_9", LookupError::Invalid),
        ("branch_other", "choice_1", LookupError::Scope),
    ] {
        assert_eq!(
            refused(ask(&session, explain(branch, event_id)))?,
            expected,
            "explaining {event_id} on {branch} was not refused as {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn a_short_page_still_discloses_what_it_could_not_observe() -> TestResult {
    let (mut session, _corpus) = setup(8192)?;
    let mut store = open(SemanticHistoryCaptureWindow {
        capture_start: 1,
        history_before_capture: Some(4),
        intervals: vec![SemanticHistoryCoverageInterval {
            from_sequence: 2,
            to_sequence: 3,
            status: SemanticHistoryCoverageStatus::Dropped,
            label: "capture_dropped".to_owned(),
        }],
    })?;
    append(
        &mut store,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
        None,
    )?;
    store.append(
        ROOT,
        gap_event("gap_2", 2),
        SemanticHistoryCausalParent::NotStated,
    )?;
    append(
        &mut store,
        "event_4",
        SemanticHistoryKind::CardPlayed,
        4,
        None,
    )?;
    session.attach_history(store)?;
    let mut spanning = page(8);
    spanning["from_sequence"] = json!(1);
    spanning["to_sequence"] = json!(4);
    let answer = answered(ask(&session, spanning))?;
    assert_eq!(
        answer["gaps"],
        json!(["dropped"]),
        "the declared gap intersecting the window is disclosed"
    );
    assert_eq!(answer["before_capture"], json!(false));
    let events = answer["events"].as_array().ok_or_else(unexpected)?;
    assert_eq!(events.len(), 3);
    let gap = &events[1];
    assert_eq!(gap["input"]["event_id"], json!("gap_2"));
    assert_eq!(gap["input"]["coverage"]["status"], json!("dropped"));
    assert_eq!(gap["input"]["coverage"]["label"], json!("capture_dropped"));
    assert_eq!(
        gap["input"]["subjects"],
        json!([]),
        "a gap names no subject it could not have observed"
    );
    assert!(
        gap["input"]["value"].is_null() && gap["input"]["reference"].is_null(),
        "a gap is never closed with an invented value or reference"
    );
    assert_eq!(gap["causal_parent"], json!({"state":"not_stated"}));
    // A window reaching before capture began says so rather than reading as a quiet run.
    let (mut later, _corpus) = setup(8192)?;
    let mut store = open(SemanticHistoryCaptureWindow::complete(4))?;
    append(
        &mut store,
        "event_4",
        SemanticHistoryKind::CardPlayed,
        4,
        None,
    )?;
    append(
        &mut store,
        "event_5",
        SemanticHistoryKind::CardPlayed,
        5,
        None,
    )?;
    later.attach_history(store)?;
    let mut earlier = page(8);
    earlier["from_sequence"] = json!(1);
    let answer = answered(ask(&later, earlier))?;
    assert_eq!(answer["before_capture"], json!(true));
    assert_eq!(answer["events"].as_array().map(Vec::len), Some(2));
    Ok(())
}

#[test]
fn the_only_history_a_provider_can_reach_is_the_one_the_owner_attached() -> TestResult {
    let (mut session, mut corpus) = setup(8192)?;
    let legal = crate::EpisodeLegalActionSet::new(
        "state-42",
        42,
        vec![crate::EpisodeLegalAction::new(
            "play:card-17",
            crate::ActionKind::PlayCard,
        )?],
    )?;
    let mut mcp = UnusedMcpPort { calls: 0 };
    // Unattached: the same question is refused by name before any store exists.
    let mut absent = HistoryAgent {
        arguments: page(4),
        answer: None,
    };
    assert_eq!(
        run_lookup_tool_loop(&mut session, &mut corpus, &mut mcp, &mut absent, &legal, 3).err(),
        Some(LookupError::MissingCapability)
    );
    assert_eq!(absent.answer, None);
    // Attached: the answer arrives through the loop, and no MCP call is made to obtain it.
    session.attach_history(card_plays(4)?)?;
    let mut agent = HistoryAgent {
        arguments: page(4),
        answer: None,
    };
    assert_eq!(
        run_lookup_tool_loop(&mut session, &mut corpus, &mut mcp, &mut agent, &legal, 3)?,
        "play:card-17"
    );
    let answer = agent.answer.ok_or_else(unexpected)?;
    assert_eq!(answer["events"].as_array().map(Vec::len), Some(4));
    assert_eq!(
        mcp.calls, 0,
        "the answer came from the attached store, not from MCP"
    );
    Ok(())
}

#[test]
fn the_archive_never_replays_a_history_answer() -> TestResult {
    let (mut session, corpus) = setup(8192)?;
    session.attach_history(card_plays(2)?)?;
    let legal = crate::EpisodeLegalActionSet::new(
        "state-42",
        42,
        vec![crate::EpisodeLegalAction::new(
            "play:card-17",
            crate::ActionKind::PlayCard,
        )?],
    )?;
    let mut agent = HistoryAgent {
        arguments: page(1),
        answer: None,
    };
    assert_eq!(
        run_lookup_replay_tool_loop(&mut session, &corpus, &mut agent, &legal, 3).err(),
        Some(LookupError::Divergence),
        "the archive retains tool transcripts, not history"
    );
    assert_eq!(agent.answer, None);
    Ok(())
}

#[test]
fn one_history_answer_must_fit_one_feedback_envelope() -> TestResult {
    let oversized = LookupFeedback::History {
        operation_id: "page_1".to_owned(),
        answer: json!({"filler":"x".repeat(7000)}),
    };
    assert_eq!(
        crate::exo_lookup_wire::feedback_value(&oversized, 7000).err(),
        Some(LookupError::Bounds),
        "an answer that cannot be delivered whole is refused, never truncated"
    );
    let bounded = LookupFeedback::History {
        operation_id: "page_1".to_owned(),
        answer: json!({"authority":HISTORY_AUTHORITY,"events":[]}),
    };
    let value = crate::exo_lookup_wire::feedback_value(&bounded, 7000)?;
    assert_eq!(value["operation_id"], json!("page_1"));
    assert_eq!(value["history"]["authority"], json!(HISTORY_AUTHORITY));
    Ok(())
}

/// One agent that asks exactly one history question and then decides.
struct HistoryAgent {
    arguments: Value,
    answer: Option<Value>,
}

impl LookupAgentPort for HistoryAgent {
    fn next_turn(&mut self, input: LookupAgentInput<'_>) -> Result<LookupTurn, LookupError> {
        match input.feedback {
            LookupFeedback::Start => history::turn(self.arguments.clone()),
            LookupFeedback::History { answer, .. } => {
                self.answer = Some(answer.clone());
                Ok(LookupTurn::Decide {
                    action_id: "play:card-17".to_owned(),
                })
            }
            // A refusal is the answer here too: the agent reports it rather than asking again.
            LookupFeedback::Error(error) => Err(error.clone()),
            _ => Err(LookupError::Divergence),
        }
    }
}

/// The MCP port the history path must never reach.
struct UnusedMcpPort {
    calls: usize,
}

impl LookupMcpPort for UnusedMcpPort {
    fn information_correlation(&self) -> Result<String, LookupError> {
        Err(LookupError::MissingCapability)
    }

    fn call_information(&mut self, _tool: &str, _request: &Value) -> Result<Vec<u8>, LookupError> {
        self.calls += 1;
        Err(LookupError::MissingCapability)
    }
}
