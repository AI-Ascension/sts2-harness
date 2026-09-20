// SPDX-License-Identifier: MIT

//! The refusals that keep a question inside the vocabulary that answered it.
//!
//! Split out of `history_tool_tests`, which owns the fixtures these tests ask through.

use super::*;
use crate::game_information::history::HISTORY_AGENT_PROFILE;
use serde_json::json;

/// One question carrying an extra key, for a deliberate refusal.
fn with(key: &str, value: Value) -> Value {
    let mut arguments = page(1);
    arguments[key] = value;
    arguments
}

/// One question with `key` removed entirely.
fn without(key: &str) -> Value {
    let mut arguments = page(1);
    if let Some(object) = arguments.as_object_mut() {
        object.remove(key);
    }
    arguments
}

#[test]
fn the_question_vocabulary_has_no_storage_coordinate() -> TestResult {
    for coordinate in [
        "path",
        "bucket",
        "artifact",
        "record_ordinal",
        "offset",
        "owner_id",
        "authority_epoch",
        "run_id",
        "store_root",
        "encoding",
    ] {
        for value in [json!("x"), json!(1), json!(null)] {
            assert_eq!(
                history::turn(with(coordinate, value.clone())).err(),
                Some(LookupError::Invalid),
                "{coordinate} was accepted as a question axis"
            );
        }
    }
    for operation in [
        "write",
        "append",
        "annotate",
        "read_storage",
        "page_all",
        "",
    ] {
        let mut arguments = page(1);
        arguments["operation"] = json!(operation);
        assert_eq!(
            history::turn(arguments).err(),
            Some(LookupError::Invalid),
            "{operation} was accepted as an operation"
        );
    }
    for operation_id in ["", "white space", "slash/inside", "non-ascii-\u{e9}"] {
        let mut arguments = page(1);
        arguments["operation_id"] = json!(operation_id);
        assert_eq!(
            history::turn(arguments).err(),
            Some(LookupError::Invalid),
            "operation id {operation_id:?} was accepted"
        );
    }
    let mut oversized_id = page(1);
    oversized_id["operation_id"] = json!("o".repeat(65));
    assert_eq!(
        history::turn(oversized_id).err(),
        Some(LookupError::Invalid)
    );
    // A branch that could be read as a host path is refused whether the question is built or
    // served: `summary` and `explain` refuse it here, and a page refuses it against the store.
    let (mut session, _corpus) = setup(8192)?;
    session.attach_history(card_plays(2)?)?;
    for branch_id in [
        "",
        "/etc/passwd",
        "..",
        "file_handle",
        "with:colon",
        "back\\slash",
    ] {
        let mut arguments = page(1);
        arguments["branch_id"] = json!(branch_id);
        assert_eq!(
            refused(ask(&session, arguments.clone()))?,
            LookupError::Invalid,
            "branch {branch_id:?} was served"
        );
        arguments["operation"] = json!("summary");
        assert_eq!(
            history::turn(arguments.clone()).err(),
            Some(LookupError::Invalid),
            "branch {branch_id:?} was accepted for a summary"
        );
        arguments["operation"] = json!("explain");
        arguments["event_id"] = json!("event_1");
        assert_eq!(
            history::turn(arguments).err(),
            Some(LookupError::Invalid),
            "branch {branch_id:?} was accepted for an explanation"
        );
    }
    for axis in [
        json!({"kind":"teleported"}),
        json!({"origin":"invented"}),
        json!({"episode":-1}),
        json!({"subject_id":"/etc/passwd"}),
        json!({"subject_id":7}),
    ] {
        let mut arguments = page(1);
        if let Some(object) = axis.as_object() {
            for (key, value) in object {
                arguments[key] = value.clone();
            }
        }
        assert_eq!(
            history::turn(arguments).err(),
            Some(LookupError::Invalid),
            "filter {axis} was accepted"
        );
    }
    for stripped in ["limit", "operation", "branch_id", "operation_id"] {
        assert_eq!(
            history::turn(without(stripped)).err(),
            Some(LookupError::Invalid),
            "a question omitting {stripped} was accepted"
        );
    }
    Ok(())
}

#[test]
fn a_canonical_request_this_boundary_did_not_write_is_never_served() -> TestResult {
    let (mut session, _corpus) = setup(8192)?;
    session.attach_history(card_plays(2)?)?;
    // The first two are canonical documents whose ask is well formed, so the profile this boundary
    // did not write is the only thing refusing them. The profile is compared by value, so a
    // near-miss name is refused for the same reason a foreign one is.
    for request in [
        json!({"profile":"other.profile.v1","ask":{"ask":"summary","branch_id":ROOT}}),
        json!({"profile":"ascension.semantic-history.v1","ask":{"ask":"summary","branch_id":ROOT}}),
        json!({"profile":"history","ask":{"ask":"summary","branch_id":ROOT}}),
        json!({"profile":"other.profile.v1","ask":{"summary":{"branch_id":ROOT}}}),
        json!({"profile":HISTORY_AGENT_PROFILE,"ask":{"drain":{"branch_id":ROOT}}}),
        json!({"profile":HISTORY_AGENT_PROFILE,"ask":{"summary":{"branch_id":ROOT,"start":0}}}),
        json!({"ask":{"summary":{"branch_id":ROOT}}}),
    ] {
        assert_eq!(
            refused(history::serve_history(
                &session,
                LookupTurn::History {
                    operation_id: "handed_over".to_owned(),
                    request: serde_json::to_vec(&request)?,
                }
            ))?,
            LookupError::Invalid,
            "{request} was served"
        );
    }
    Ok(())
}

#[test]
fn a_continuation_the_store_outran_diverges_rather_than_answering() -> TestResult {
    let (mut session, _corpus) = setup(8192)?;
    session.attach_history(card_plays(4)?)?;
    let cursor = answered(ask(&session, page(2)))?["continuation"].clone();
    // The history moved on. The continuation describes a generation that no longer exists.
    let store = session.history.as_mut().ok_or_else(unexpected)?;
    append(store, "event_5", SemanticHistoryKind::CardPlayed, 5, None)?;
    let mut resumed = page(2);
    resumed["operation_id"] = json!("page_2");
    resumed["continuation"] = cursor;
    assert_eq!(refused(ask(&session, resumed))?, LookupError::Divergence);
    // A cursor minted for another question answers a different question, so it is refused too.
    let mut other = page(2);
    other["origin"] = json!("native");
    let elsewhere = answered(ask(&session, other))?["continuation"].clone();
    let mut borrowed = page(4);
    borrowed["operation_id"] = json!("page_3");
    borrowed["continuation"] = elsewhere;
    assert_eq!(refused(ask(&session, borrowed))?, LookupError::Divergence);
    Ok(())
}
