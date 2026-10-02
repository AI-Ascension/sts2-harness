// SPDX-License-Identifier: MIT

//! Bounds the fair-play sandbox shares with the runtime-v3 contract it mirrors.
//!
//! Every case here pins both verdicts. A test that pinned only the accepted case
//! would not catch the byte-versus-character drift this file exists to prevent.

#![allow(clippy::expect_used)]

use super::{MAX_TEXT_CHARACTERS, SandboxError, SanitizedObservation};
use serde_json::{Value, json};

/// A minimal observation that the sandbox admits, used as the control case.
fn observation() -> Value {
    json!({
        "state_id":"state-1", "generation":1, "visible_seed":null,
        "player":{"hp":50,"max_hp":50,"energy":3,"gold":99,
            "hand":[],"deck":[],"discard":[],"exhaust":[]},
        "state":{"state":"map","node_id":"start","options":[]},
        "legal_actions":[{"action_id":"move-1",
            "action":{"kind":"select_map_node","node_id":"start"}}]
    })
}

/// The same observation with a described offered entry whose `name` is `text`.
fn with_offered_name(name: &str) -> Value {
    let mut value = observation();
    value["state"]["options"] = json!([{
        "choice_id":"reward-1",
        "name":name,
        "description":"a described reward"
    }]);
    value
}

fn admit(value: Value) -> Result<(), SandboxError> {
    SanitizedObservation::new(value).map(|_| ())
}

#[test]
fn bare_identity_options_are_admitted() {
    // Control: the fixture shape is known good before any bound is varied.
    let mut value = observation();
    value["state"]["options"] = json!(["reward-1"]);
    assert_eq!(admit(value), Ok(()));
}

#[test]
fn offered_attribute_text_is_bounded_in_characters_not_bytes() {
    // 512 characters, 1024 UTF-8 bytes. A byte bound refuses this; the contract
    // admits it, because #/$defs/offered_attribute maxLength counts characters.
    let accented = "é".repeat(MAX_TEXT_CHARACTERS);
    assert_eq!(accented.chars().count(), MAX_TEXT_CHARACTERS);
    assert!(accented.len() > MAX_TEXT_CHARACTERS);
    assert_eq!(admit(with_offered_name(&accented)), Ok(()));
}

#[test]
fn offered_attribute_text_over_the_character_bound_is_refused() {
    let accented = "é".repeat(MAX_TEXT_CHARACTERS + 1);
    assert!(accented.len() > MAX_TEXT_CHARACTERS + 1);
    assert_eq!(
        admit(with_offered_name(&accented)),
        Err(SandboxError::InvalidText)
    );
}

#[test]
fn ascii_offered_attribute_text_is_bounded_at_the_same_limit() {
    // The bound is one limit, not one per encoding: ASCII and non-ASCII agree.
    assert_eq!(
        admit(with_offered_name(&"a".repeat(MAX_TEXT_CHARACTERS))),
        Ok(())
    );
    assert_eq!(
        admit(with_offered_name(&"a".repeat(MAX_TEXT_CHARACTERS + 1))),
        Err(SandboxError::InvalidText)
    );
}

#[test]
fn identities_stay_bounded_in_bytes_and_reject_non_ascii() {
    // Identities are ASCII-only by contract, so a byte bound is equivalent to a
    // character bound here and must stay that way.
    let mut value = observation();
    value["state_id"] = json!("a".repeat(512));
    assert_eq!(admit(value), Ok(()));

    let mut over = observation();
    over["state_id"] = json!("a".repeat(513));
    assert_eq!(admit(over), Err(SandboxError::InvalidText));

    let mut non_ascii = observation();
    non_ascii["state_id"] = json!("é");
    assert_eq!(admit(non_ascii), Err(SandboxError::InvalidText));
}

#[test]
fn choice_contents_admits_the_contract_maximum() {
    // #/$defs/disclosed_set declares maxItems 256. The sandbox previously capped
    // this at 32, so a producer that trusted the schema had 33..256 refused.
    let contents: Vec<Value> = (0..256)
        .map(|index| json!({"choice_id":format!("choice-{index}")}))
        .collect();
    let mut value = observation();
    value["state"]["options"] = json!([{
        "choice_id":"reward-1",
        "name":"a described reward",
        "description":"what it would present next",
        "contents":contents
    }]);
    assert_eq!(admit(value), Ok(()));
}

#[test]
fn choice_contents_over_the_contract_maximum_is_refused() {
    let contents: Vec<Value> = (0..257)
        .map(|index| json!({"choice_id":format!("choice-{index}")}))
        .collect();
    let mut value = observation();
    value["state"]["options"] = json!([{
        "choice_id":"reward-1",
        "name":"a described reward",
        "description":"what it would present next",
        "contents":contents
    }]);
    assert_eq!(admit(value), Err(SandboxError::InvalidCollection));
}
