// SPDX-License-Identifier: MIT

//! Opt-in, bounded semantic evaluation. No game rules, networking, or mutation authority.

use serde_json::{Value, json};
use sts2_harness::{ACTION_QUESTION, SystemOneOption, describe_action};

#[path = "jev_tactical_request.rs"]
mod request;
#[path = "jev_tactical_response.rs"]
mod response;
#[path = "jev_tactical_selection.rs"]
mod selection;

pub(super) use request::prepare;
pub(super) use selection::evaluate;

pub(super) const PROFILE: &str = "jev-tactical-v1";
pub(super) const MAX_OPTIONS: usize = 24;
const MAX_QUESTIONS: usize = 1 + MAX_OPTIONS * 7;
const MAX_BODY_BYTES: usize = 60 * 1024;
const MAX_STATE_QUESTION_BYTES: usize = 24 * 1024;
const LEVELS: [&str; 3] = [
    "Poor contribution",
    "Mixed or neutral contribution",
    "Strong contribution",
];

// Proposed heuristic weights, not fitted values or probabilities of winning.
const AXES: [(&str, &str, f64); 6] = [
    (
        "immediate",
        "How much does this action advance the immediate objective?",
        3.0,
    ),
    (
        "threat",
        "How much does this action reduce important future threats?",
        3.0,
    ),
    (
        "setup",
        "How much does this action enable useful follow-up actions?",
        2.0,
    ),
    (
        "resource",
        "How well does this action preserve valuable future resources?",
        2.0,
    ),
    (
        "strategy",
        "How well does this action fit the current run objective?",
        2.0,
    ),
    (
        "safety",
        "How well does this action avoid a severe immediate downside?",
        4.0,
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Error {
    Catalog,
    Request,
    Answers,
    Distribution,
    Gate,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Catalog => "tactical catalog is invalid",
            Self::Request => "tactical request is invalid",
            Self::Answers => "tactical answer set is invalid",
            Self::Distribution => "tactical answer distribution is invalid",
            Self::Gate => "tactical confidence gate is invalid",
        })
    }
}

impl std::error::Error for Error {}

pub(super) struct Prepared {
    pub body: Value,
    pub applied: bool,
    pub fallback_reason: Option<&'static str>,
}

/// Never fold or prune the tactical candidate set. Large sets use the legacy lane explicitly.
pub(super) fn catalog_options(observation: &Value, catalog: &[String]) -> Vec<SystemOneOption> {
    let entries = observation["legal_actions"].as_array();
    catalog
        .iter()
        .map(|id| {
            let entry = entries.and_then(|items| {
                items
                    .iter()
                    .find(|entry| entry["action_id"].as_str() == Some(id.as_str()))
            });
            SystemOneOption {
                id: id.clone(),
                description: entry
                    .map_or_else(|| id.clone(), |item| describe_action(item, observation)),
            }
        })
        .collect()
}

pub(super) fn validate_catalog(catalog: &[String]) -> Result<(), Error> {
    if catalog.is_empty() || catalog.len() > 256 {
        return Err(Error::Catalog);
    }
    for (index, id) in catalog.iter().enumerate() {
        if id.is_empty()
            || id.len() > 512
            || id.chars().any(char::is_control)
            || catalog[..index].contains(id)
        {
            return Err(Error::Catalog);
        }
    }
    Ok(())
}

fn key(index: usize, axis: &str) -> String {
    format!("tactical_{index}_{axis}")
}

fn refusal(reason: &str) -> Value {
    // Deliberately omit candidate_action_id: the runner must not force an underinformed action.
    json!({"decision": "reobserve", "rationale": format!("bridge-authored tactical evidence: {reason}")})
}

#[cfg(test)]
#[path = "jev_tactical_tests.rs"]
mod tests;
