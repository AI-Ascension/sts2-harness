// SPDX-License-Identifier: MIT

//! Maps one System One answer to one terminal bridge decision.
//!
//! The provider returns a chosen option, a probability for every option, and a confidence derived
//! from the shape of that distribution. Three things follow, and this module exists to make all
//! three explicit rather than incidental.
//!
//! **The choice is re-checked against the catalog.** The `criteria` keys constrain the answer
//! structurally, but an answer naming an action the host did not offer is refused here rather than
//! trusted, because the host is the authority on legality and a provider is not.
//!
//! **Low confidence becomes `reobserve`, not a guess.** The bridge contract already carries four
//! terminal decisions, so an answer whose probability mass is spread has somewhere honest to go. The
//! gate is a parameter, not a constant buried in a branch, so an operator can require more certainty
//! for an irreversible action than for a card play.
//!
//! **The rationale is bridge-authored and says so.** This provider generates no text. Any sentence
//! attached to its decision is written here, so it is composed from the distribution — the chosen
//! option, its probability, the runner-up, and the confidence — and labelled. A fluent sentence
//! presented as model reasoning would be a fabricated record.

use serde_json::{Value, json};

/// Confidence at or above which an action is returned rather than a re-observation.
///
/// `proposed`: no run has measured how this provider's confidence distributes over real combat
/// states, so this is a starting value to be revised against evidence, not a calibrated threshold.
pub const DEFAULT_CONFIDENCE_GATE: f64 = 0.55;

/// Largest rationale the decision contract accepts.
const MAX_RATIONALE_BYTES: usize = 512;

/// Why a System One answer could not become a decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionError {
    /// The response carried no answer under the action question.
    MissingAnswer,
    /// The answer was not a choice answer.
    WrongAnswerType,
    /// The answer carried no chosen option.
    MissingChoice,
    /// The chosen option was not one of the options that were presented.
    OutOfCatalog,
    /// The answer carried no usable confidence value.
    MissingConfidence,
}

impl DecisionError {
    /// Stable machine-readable code for records and diagnostics.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::MissingAnswer => "missing answer",
            Self::WrongAnswerType => "unexpected answer type",
            Self::MissingChoice => "missing choice",
            Self::OutOfCatalog => "choice is outside the presented options",
            Self::MissingConfidence => "missing or invalid confidence",
        }
    }
}

impl std::fmt::Display for DecisionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for DecisionError {}

/// Maps the answer under `question` to one terminal decision object.
///
/// Returns an `action` decision at or above `gate`, and a `reobserve` decision below it. The
/// returned object carries only keys the decision contract allows.
///
/// # Errors
///
/// Returns a [`DecisionError`] when the answer is absent, of the wrong type, missing its choice or
/// confidence, or names an option that was not presented.
pub fn map_decision(
    response: &Value,
    question: &str,
    options: &[String],
    gate: f64,
) -> Result<Value, DecisionError> {
    let answer = response
        .get("answers")
        .and_then(|answers| answers.get(question))
        .ok_or(DecisionError::MissingAnswer)?;
    if answer.get("type").and_then(Value::as_str) != Some("choice") {
        return Err(DecisionError::WrongAnswerType);
    }
    let choice = answer
        .get("choice")
        .and_then(Value::as_str)
        .ok_or(DecisionError::MissingChoice)?;
    if !options.iter().any(|option| option == choice) {
        return Err(DecisionError::OutOfCatalog);
    }
    let confidence = answer
        .get("confidence")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        .ok_or(DecisionError::MissingConfidence)?;

    let probabilities = answer.get("probabilities");
    let rationale = rationale(choice, confidence, probabilities);
    if confidence < gate {
        return Ok(json!({"decision": "reobserve", "rationale": rationale}));
    }
    Ok(json!({
        "decision": "action",
        "action_id": choice,
        "rationale": rationale,
        "confidence": percent(confidence),
    }))
}

/// Converts a unit confidence into the integer percentage the decision contract carries.
fn percent(confidence: f64) -> u64 {
    let scaled = (confidence * 100.0).round();
    if scaled <= 0.0 {
        return 0;
    }
    if scaled >= 100.0 {
        return 100;
    }
    // Finite and strictly inside 0..100 here, so the conversion is exact.
    scaled as u64
}

/// Composes the bridge-authored evidence string.
///
/// Shrinks rather than fails: the runner-up is dropped, then the probability, then the chosen
/// identifier, so a long identifier cannot push the contract's 512-byte bound.
fn rationale(choice: &str, confidence: f64, probabilities: Option<&Value>) -> String {
    let chosen = probabilities
        .and_then(|values| values.get(choice))
        .and_then(Value::as_f64);
    let with_probability =
        chosen.map(|chosen| format!("bridge-authored evidence: chose {choice} at p={chosen:.2}"));
    let with_runner_up = with_probability
        .clone()
        .zip(runner_up(choice, probabilities))
        .map(|(head, (name, value))| {
            format!("{head}, runner-up {name} at p={value:.2}, confidence {confidence:.2}")
        });
    let candidates = [
        with_runner_up,
        with_probability.map(|head| format!("{head}, confidence {confidence:.2}")),
        Some(format!(
            "bridge-authored evidence: chose {choice}, confidence {confidence:.2}"
        )),
        Some(format!(
            "bridge-authored evidence: confidence {confidence:.2}"
        )),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|text| !text.is_empty() && text.len() <= MAX_RATIONALE_BYTES && printable(text))
        .unwrap_or_else(|| String::from("bridge-authored evidence"))
}

/// The highest-probability option other than the chosen one.
fn runner_up<'a>(choice: &str, probabilities: Option<&'a Value>) -> Option<(&'a str, f64)> {
    probabilities?
        .as_object()?
        .iter()
        .filter(|(name, _)| name.as_str() != choice)
        .filter_map(|(name, value)| Some((name.as_str(), value.as_f64()?)))
        .max_by(|left, right| left.1.total_cmp(&right.1))
}

/// Whether every byte is printable ASCII, which the decision contract requires of a rationale.
fn printable(value: &str) -> bool {
    value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
}

#[cfg(test)]
#[path = "systemone_decision_tests.rs"]
mod tests;
