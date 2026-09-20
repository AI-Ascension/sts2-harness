// SPDX-License-Identifier: MIT

//! The history tool an agent asks through, consumed at its own boundary.
//!
//! `sts2-harness#128` requirement 4 lets a provider ask history through a harness-owned agent tool
//! port, and forbids the MCP game adapter from bypassing to arbitrary artifact storage or
//! reverse-calling the harness. The port is `SemanticHistoryAgentPort`, and the closed question
//! vocabulary in front of it is `history::turn`; both live inside the crate, so the questions this
//! suite asks are the questions the relay can actually ask, and a guard deleted here fails a test
//! rather than quietly widening the surface.
//!
//! This module owns the grant and the page: the attachment the owner made, the scope it is
//! re-checked against on every read, the continuation that resumes a page and the bounds one
//! question cannot widen. The vocabulary's own refusals live in `vocabulary_tests`, and the answers
//! it gives — explanations, disclosed gaps, the loop, the archive and the feedback envelope — in
//! `answer_tests`.
//!
//! Only the vocabulary and the answers are covered here. The additive profile's advertisement, its
//! frame pins and the process-level profile gate are covered where they are public:
//! `tests/semantic_history_agent_tool.rs`.

use serde_json::json;

use super::*;
use crate::game_information::history;
use crate::game_information::history::HISTORY_AUTHORITY;
use crate::semantic_history::{
    SemanticHistoryBinding, SemanticHistoryCaptureWindow, SemanticHistoryCausalParent,
    SemanticHistoryCoverage, SemanticHistoryError, SemanticHistoryEventInput, SemanticHistoryKind,
    SemanticHistoryNamespace, SemanticHistoryOrigin, SemanticHistoryReference,
    SemanticHistoryStore, SemanticHistorySubject, SemanticHistorySubjectRole,
};

#[path = "game_information_history_answer_tests.rs"]
mod answer_tests;
#[path = "game_information_history_vocabulary_tests.rs"]
mod vocabulary_tests;

const ROOT: &str = "branch_root";

/// The store binding the session `setup` builds is the owner of.
fn store_binding() -> SemanticHistoryBinding {
    SemanticHistoryBinding {
        project_id: "project-1".to_owned(),
        run_id: "run-1".to_owned(),
        agent_id: "agent-1".to_owned(),
        game_profile: "fair-play-v1".to_owned(),
        content_manifest_id: "synthetic-content-1".to_owned(),
        locale: "en".to_owned(),
        authority_epoch: 1,
    }
}

fn open(
    window: SemanticHistoryCaptureWindow,
) -> Result<SemanticHistoryStore, SemanticHistoryError> {
    SemanticHistoryStore::open(store_binding(), ROOT, window)
}

/// One observed event naming the end its kind requires and nothing it does not.
fn event(event_id: &str, kind: SemanticHistoryKind, sequence: u64) -> SemanticHistoryEventInput {
    SemanticHistoryEventInput {
        event_id: event_id.to_owned(),
        kind,
        sequence,
        episode: 1,
        authority_epoch: 1,
        origin: SemanticHistoryOrigin::Native,
        subjects: vec![SemanticHistorySubject {
            role: SemanticHistorySubjectRole::Actor,
            namespace: SemanticHistoryNamespace::LiveInstance,
            identity: "instance_hero".to_owned(),
        }],
        value: None,
        reference: (kind == SemanticHistoryKind::CardPlayed).then(|| SemanticHistoryReference {
            entity_kind: "card".to_owned(),
            namespaced_id: "synthetic_strike".to_owned(),
        }),
        coverage: SemanticHistoryCoverage::captured(),
    }
}

fn append(
    store: &mut SemanticHistoryStore,
    event_id: &str,
    kind: SemanticHistoryKind,
    sequence: u64,
    parent: Option<&str>,
) -> Result<(), SemanticHistoryError> {
    let causal_parent = parent.map_or(SemanticHistoryCausalParent::NotStated, |parent| {
        SemanticHistoryCausalParent::Stated {
            event_id: parent.to_owned(),
        }
    });
    store.append(ROOT, event(event_id, kind, sequence), causal_parent)?;
    Ok(())
}

/// A store holding `count` plain card plays, with sequences `1..=count`.
fn card_plays(count: u64) -> Result<SemanticHistoryStore, SemanticHistoryError> {
    let mut store = open(SemanticHistoryCaptureWindow::complete(1))?;
    for sequence in 1..=count {
        append(
            &mut store,
            &format!("event_{sequence}"),
            SemanticHistoryKind::CardPlayed,
            sequence,
            None,
        )?;
    }
    Ok(store)
}

/// Asks one question exactly as the relay does, and returns the feedback it produced.
fn ask(session: &LookupSession, arguments: Value) -> LookupFeedback {
    match history::turn(arguments) {
        Ok(turn) => history::serve_history(session, turn),
        Err(error) => LookupFeedback::Error(error),
    }
}

fn page(limit: usize) -> Value {
    json!({"operation":"page","operation_id":"page_1","branch_id":ROOT,"limit":limit})
}

fn unexpected() -> serde_json::Error {
    <serde_json::Error as serde::de::Error>::custom("unexpected history feedback")
}

fn answered(feedback: LookupFeedback) -> Result<Value, serde_json::Error> {
    match feedback {
        LookupFeedback::History { answer, .. } => Ok(answer),
        _ => Err(unexpected()),
    }
}

fn refused(feedback: LookupFeedback) -> Result<LookupError, serde_json::Error> {
    match feedback {
        LookupFeedback::Error(error) => Ok(error),
        _ => Err(unexpected()),
    }
}

fn explain(branch_id: &str, event_id: &str) -> Value {
    json!({"operation":"explain","operation_id":"explain_1","branch_id":branch_id,"event_id":event_id})
}

#[test]
fn a_session_the_owner_attached_no_history_to_has_no_history_tool() -> TestResult {
    let (session, _corpus) = setup(8192)?;
    assert_eq!(
        refused(ask(&session, page(1)))?,
        LookupError::MissingCapability,
        "an unattached session refuses by capability name, not with an empty history"
    );
    Ok(())
}

#[test]
fn the_page_it_answers_carries_the_continuation_that_resumes_it() -> TestResult {
    let (mut session, _corpus) = setup(8192)?;
    session.attach_history(card_plays(10)?)?;
    let first = answered(ask(&session, page(8)))?;
    assert_eq!(first["authority"], json!(HISTORY_AUTHORITY));
    assert_eq!(first["before_capture"], json!(false));
    assert_eq!(first["gaps"], json!([]));
    assert_eq!(first["events"].as_array().map(Vec::len), Some(8));
    assert_eq!(first["events"][0]["input"]["event_id"], json!("event_1"));
    assert_eq!(first["events"][0]["input"]["kind"], json!("card_played"));
    assert_eq!(first["events"][0]["input"]["origin"], json!("native"));
    assert_eq!(first["events"][0]["branch_id"], json!(ROOT));
    let cursor = first["continuation"].clone();
    assert!(
        cursor["query_digest"]
            .as_str()
            .is_some_and(|d| !d.is_empty()),
        "the continuation is bound to the question that minted it"
    );
    assert_eq!(cursor["generation"], json!(10));
    assert_eq!(cursor["next_sequence"], json!(9));
    // The continuation is spendable, not merely handed out: the relay asks again and is answered.
    let mut resumed = page(8);
    resumed["operation_id"] = json!("page_2");
    resumed["continuation"] = cursor;
    let second = answered(ask(&session, resumed))?;
    assert_eq!(second["events"].as_array().map(Vec::len), Some(2));
    assert_eq!(second["events"][0]["input"]["event_id"], json!("event_9"));
    assert!(second["continuation"].is_null());
    // A filter that matches nothing is still a deliverable answer rather than a refusal.
    let mut filtered = page(8);
    filtered["origin"] = json!("imported");
    assert_eq!(answered(ask(&session, filtered))?["events"], json!([]));
    let summary = answered(ask(
        &session,
        json!({"operation":"summary","operation_id":"summary_1","branch_id":ROOT}),
    ))?;
    assert_eq!(summary["authority"], json!(HISTORY_AUTHORITY));
    assert_eq!(summary["total"], json!(10));
    assert_eq!(summary["captured"], json!(10));
    assert_eq!(summary["gaps"], json!(0));
    assert_eq!(summary["first_sequence"], json!(1));
    assert_eq!(summary["last_sequence"], json!(10));
    Ok(())
}

#[test]
fn a_store_serving_another_owner_is_refused_at_attach_and_again_at_read() -> TestResult {
    for drifted in [
        SemanticHistoryBinding {
            run_id: "run-2".to_owned(),
            ..store_binding()
        },
        SemanticHistoryBinding {
            authority_epoch: 2,
            ..store_binding()
        },
        SemanticHistoryBinding {
            game_profile: "research".to_owned(),
            ..store_binding()
        },
    ] {
        let (mut session, _corpus) = setup(8192)?;
        let window = SemanticHistoryCaptureWindow::complete(1);
        assert_eq!(
            session
                .attach_history(SemanticHistoryStore::open(drifted, ROOT, window)?)
                .err(),
            Some(LookupError::Scope),
            "a store for another run, epoch or profile is never attached"
        );
    }
    let (mut session, _corpus) = setup(8192)?;
    session.attach_history(card_plays(2)?)?;
    assert_eq!(
        answered(ask(&session, page(1)))?["events"][0]["input"]["sequence"],
        json!(1)
    );
    // The grant is re-checked on every read, so a session that outlived an owner change stops.
    session.binding.authority_epoch = 2;
    assert_eq!(refused(ask(&session, page(1)))?, LookupError::Scope);
    // A second attachment would replace the first, so it is refused instead.
    session.binding.authority_epoch = 1;
    assert_eq!(
        session.attach_history(card_plays(1)?).err(),
        Some(LookupError::Scope)
    );
    Ok(())
}

#[test]
fn one_question_cannot_widen_the_page_or_the_walk_it_was_granted() -> TestResult {
    for limit in [0, 9, 256, usize::MAX] {
        assert_eq!(
            history::turn(page(limit)).err(),
            Some(LookupError::Bounds),
            "a page bound of {limit} was admitted"
        );
    }
    let mut inverted = page(1);
    inverted["from_sequence"] = json!(5);
    inverted["to_sequence"] = json!(4);
    assert_eq!(history::turn(inverted).err(), Some(LookupError::Bounds));
    for limits in [
        json!({"max_depth":0,"max_visits":1}),
        json!({"max_depth":17,"max_visits":1}),
        json!({"max_depth":1,"max_visits":0}),
        json!({"max_depth":1,"max_visits":257}),
    ] {
        let mut arguments = explain(ROOT, "event_1");
        arguments["limits"] = limits.clone();
        assert_eq!(
            history::turn(arguments).err(),
            Some(LookupError::Bounds),
            "walk bounds {limits} were admitted"
        );
    }
    // The walk bounds are bounded even when the whole envelope is small enough to parse.
    let mut huge = page(1);
    huge["pad"] = json!("x".repeat(crate::exo_lookup_wire::EXO_LOOKUP_TOOL_BYTES));
    assert_eq!(
        history::turn(huge).err(),
        Some(LookupError::Bounds),
        "an oversized arguments envelope is refused before it is read"
    );
    Ok(())
}
