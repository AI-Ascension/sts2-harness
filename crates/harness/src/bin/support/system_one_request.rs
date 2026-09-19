// SPDX-License-Identifier: MIT

//! Builds the provider request body from one bridge request.
//!
//! These read the harness request rather than the transport: the option set that is presented, the
//! arithmetic derived from the observation, and the catalog and constraints the request carries.
//! They live apart from the executable so the bridge stays one reviewable boundary around process
//! execution and decision shape rather than mixing request reading into that plumbing.

use serde_json::Value;
use sts2_harness::{
    DerivedExactFacts, OptionSelection, SystemOneOption, describe_action, ollama_user_content,
};

/// Builds the option set to present, each carrying a description composed from the observation.
///
/// Falls back to the whole catalog when the selection presented nothing usable, so a selection that
/// cannot read this observation costs the run nothing.
pub(super) fn present(
    selection: &OptionSelection,
    observation: &Value,
    catalog: &[String],
) -> Vec<SystemOneOption> {
    let entries = observation
        .get("legal_actions")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let describe = |action_id: &str| -> String {
        entries
            .iter()
            .find(|entry| entry.get("action_id").and_then(Value::as_str) == Some(action_id))
            .map(|entry| describe_action(entry, observation))
            .unwrap_or_else(|| action_id.to_owned())
    };
    if selection.presented.is_empty() {
        return catalog
            .iter()
            .map(|id| SystemOneOption {
                id: id.clone(),
                description: describe(id),
            })
            .collect();
    }
    selection
        .presented
        .iter()
        .map(|option| SystemOneOption {
            id: option.action_id.clone(),
            description: describe(&option.action_id),
        })
        .collect()
}

/// Renders the state, adding the facts that follow exactly from this observation.
///
/// The facts are derived, never fetched: incoming damage is the sum the host's own revealed intents
/// state, and each one is omitted when the observation does not support it exactly. They are added
/// because the arithmetic is the part a System One model is documented not to do, and the state
/// already carries every term of it.
pub(super) fn state_with_derived_facts(
    request: &Value,
    observation: &Value,
) -> Result<String, Box<dyn std::error::Error>> {
    let rendered = ollama_user_content(request)?;
    let facts = DerivedExactFacts::from_observation(observation);
    let Ok(mut value) = serde_json::from_str::<Value>(&rendered) else {
        return Ok(rendered);
    };
    let Some(object) = value.as_object_mut() else {
        return Ok(rendered);
    };
    object.insert(String::from("derived_exact"), serde_json::to_value(facts)?);
    Ok(value.to_string())
}

/// Reads the host-generated action catalog from the request.
pub(super) fn catalog(request: &Value) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let ids = request["legal_action_ids"]
        .as_array()
        .ok_or("missing catalog")?;
    if ids.is_empty() || ids.len() > 256 || ids.iter().any(|value| !value.is_string()) {
        return Err("invalid catalog".into());
    }
    Ok(ids
        .iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect())
}

/// Reads the hard constraints from the request, tolerating their absence.
pub(super) fn constraints(request: &Value) -> Vec<String> {
    request["hard_constraints"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}
