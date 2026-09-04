// SPDX-License-Identifier: MIT

//! Structural requirements checks, not a game-state or runtime-safety oracle.
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::io;
use std::path::Path;

use serde_json::Value;

use crate::{load_records, required_string};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub(crate) fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidData, message).into())
    }
}

pub(crate) fn ids<'a>(rows: &'a [Value], key: &str) -> Result<BTreeSet<&'a str>> {
    let values = rows
        .iter()
        .map(|row| required_string(row, key))
        .collect::<Result<BTreeSet<_>>>()?;
    require(values.len() == rows.len(), "duplicate inventory identity")?;
    Ok(values)
}

pub(crate) fn strings(value: &Value) -> Result<BTreeSet<&str>> {
    let Some(values) = value.as_array() else {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "expected reference array").into());
    };
    let result = values
        .iter()
        .map(|value| {
            value.as_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "expected string reference").into()
            })
        })
        .collect::<Result<BTreeSet<_>>>()?;
    require(result.len() == values.len(), "duplicate reference")?;
    Ok(result)
}

pub(crate) fn validate(root: &Path, states: &[Value]) -> Result<()> {
    let observations = load_records(root, "observations.json")?;
    let actions = load_records(root, "actions.json")?;
    let transitions = load_records(root, "transitions.json")?;
    validate_joins(states, &observations, &actions, &transitions)?;
    crate::digests::validate_digests(root)?;
    crate::fixtures::validate_fixtures(root, states)?;
    Ok(())
}

pub(crate) fn validate_joins(
    states: &[Value],
    observations: &[Value],
    actions: &[Value],
    transitions: &[Value],
) -> Result<()> {
    let state_ids = ids(states, "state_id")?;
    ids(observations, "record_id")?;
    ids(actions, "action_id")?;
    ids(transitions, "transition_id")?;
    for rows in [states, observations, actions, transitions] {
        for row in rows {
            require(
                row["evidence_status"] == "proposed"
                    && row["target_build_observed"] == false
                    && row["live_capture"] == false,
                "requirements evidence unexpectedly upgraded",
            )?;
        }
    }
    for row in observations.iter().chain(actions) {
        require(
            state_ids.contains(required_string(row, "state_id")?),
            "dangling state reference",
        )?;
    }
    for row in observations {
        if row["access_class"] == "PRIVILEGED" {
            require(
                row["production_observation"] == false
                    && row["legal_action_input"] == false
                    && row["privileged"] == true,
                "privileged field admitted",
            )?;
        }
    }
    let action_owners: BTreeMap<_, _> = actions
        .iter()
        .map(|row| {
            Ok((
                required_string(row, "action_id")?,
                required_string(row, "state_id")?,
            ))
        })
        .collect::<Result<_>>()?;
    for transition in transitions {
        let source = required_string(transition, "source_state_id")?;
        let target = required_string(transition, "target_state_id")?;
        require(
            state_ids.contains(source) && state_ids.contains(target),
            "dangling transition endpoint",
        )?;
        require(
            action_owners.get(required_string(transition, "action_id")?) == Some(&source),
            "transition action belongs to another state",
        )?;
    }
    for state in states {
        validate_state_joins(state, observations, actions, transitions)?;
    }
    Ok(())
}

fn validate_state_joins(
    state: &Value,
    observations: &[Value],
    actions: &[Value],
    transitions: &[Value],
) -> Result<()> {
    let id = required_string(state, "state_id")?;
    let owned_actions: Vec<_> = actions
        .iter()
        .filter(|row| row["state_id"] == id)
        .cloned()
        .collect();
    let owned_transitions: Vec<_> = transitions
        .iter()
        .filter(|row| row["source_state_id"] == id)
        .cloned()
        .collect();
    require(
        strings(&state["legal_actions"])? == ids(&owned_actions, "action_id")?,
        "state action join drift",
    )?;
    require(
        strings(&state["expected_transitions"])? == ids(&owned_transitions, "transition_id")?,
        "state transition join drift",
    )?;
    for (direction, endpoint, opposite) in [
        ("successors", "source_state_id", "target_state_id"),
        ("predecessors", "target_state_id", "source_state_id"),
    ] {
        let expected: BTreeSet<_> = transitions
            .iter()
            .filter(|row| row[endpoint] == id)
            .map(|row| required_string(row, opposite))
            .collect::<Result<_>>()?;
        require(
            strings(&state["identity"][direction])? == expected,
            "state graph join drift",
        )?;
    }
    let fields: BTreeSet<_> = observations
        .iter()
        .filter(|row| row["state_id"] == id)
        .map(|row| required_string(row, "field_id"))
        .collect::<Result<_>>()?;
    for key in [
        "required_fields",
        "on_demand_fields",
        "historical_fields",
        "derived_fields",
        "estimated_fields",
        "unavailable_fields",
    ] {
        require(
            strings(&state["observations"][key])?.is_subset(&fields),
            "dangling state field reference",
        )?;
    }
    for action in &owned_actions {
        let action_id = required_string(action, "action_id")?;
        let expected: BTreeSet<_> = owned_transitions
            .iter()
            .filter(|row| row["action_id"] == action_id)
            .map(|row| required_string(row, "target_state_id"))
            .collect::<Result<_>>()?;
        require(
            strings(&action["expected_successor_state_ids"])? == expected,
            "action successor join drift",
        )?;
    }
    Ok(())
}

pub(crate) fn read_json(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
