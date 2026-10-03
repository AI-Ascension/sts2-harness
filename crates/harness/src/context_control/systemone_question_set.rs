// SPDX-License-Identifier: MIT

//! Builds one System One request carrying several `choice` questions.
//!
//! A System One provider evaluates every question in a call against the same state, so asking the
//! reward and the card inside it together costs one exchange rather than two. This module owns that
//! pairing: the single-question builder beside it is one caller of it, so the two cannot disagree
//! about what a legal question set is or how the shared byte ceiling is applied.
//!
//! The pairing is deliberately strict. A question is admitted only when something reads its
//! answer, so the set is never padded with a question no code consumes, and every answer is checked
//! back against the exact option set that was presented under its own name. A provider that names
//! an option that was never offered, or answers one question in place of another, is refused rather
//! than repaired: the host is the authority on legality and a provider is not.

use serde_json::{Value, json};

use super::systemone_request::{
    MAX_STATE_AND_QUESTION_BYTES, SystemOneOption, SystemOneRequestError, printable,
    validate_model, validate_options,
};

/// Largest number of questions one request may carry.
///
/// One `choice` per question keeps a set small enough to read and to rank; a bound exists so a
/// caller cannot turn the map into an unbounded token spend.
pub const MAX_QUESTIONS: usize = 8;

/// One `choice` question: the name it answers under, the options it chooses between, and what the
/// model is told before it chooses.
///
/// Grouping the three keeps the pairing of a name to the option set and instruction it was asked
/// with unbreakable at the type level. The name is not an identifier the caller gets to choose
/// freely: [`validate_questions`] still refuses an empty, duplicated or non-printable one, because
/// the name is what an answer resolves to and an unresolvable name is an unconsumed question.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemOneQuestion<'a> {
    /// The key this question answers under in `answers`.
    pub name: &'a str,
    /// The options presented for this question.
    pub options: &'a [SystemOneOption],
    /// What the model is told before it chooses.
    pub instructions: &'a str,
}

impl<'a> SystemOneQuestion<'a> {
    /// Pairs a question name with the option set and instruction it is asked with.
    #[must_use]
    pub const fn new(name: &'a str, options: &'a [SystemOneOption], instructions: &'a str) -> Self {
        Self {
            name,
            options,
            instructions,
        }
    }
}

/// Builds one request body carrying every supplied `choice` question.
///
/// The questions are ordered by the caller, and `serde_json` orders the keys of the resulting map,
/// so a fixed input serializes byte-stably and the question set can still be digested. The budget
/// check is taken over the longest single question rather than the whole set, which is the term
/// that shares the ceiling with the state: asking several questions does not make any one of them
/// larger.
///
/// # Errors
///
/// Returns a [`SystemOneRequestError`] for an empty, oversized, or malformed input. The builder
/// never truncates the state to fit: a smaller state is the caller's decision to make, not a silent
/// one.
pub fn build_choice_questions_request(
    model: &str,
    state: &str,
    questions: &[SystemOneQuestion<'_>],
) -> Result<Value, SystemOneRequestError> {
    validate_model(model)?;
    if state.is_empty() {
        return Err(SystemOneRequestError::EmptyState);
    }
    validate_questions(questions)?;
    let mut map = serde_json::Map::new();
    for question in questions {
        map.insert(
            question.name.to_owned(),
            json!({
                "type": "choice",
                "instructions": question.instructions,
                "criteria": criteria(question.options),
            }),
        );
    }
    let questions = Value::Object(map);
    let longest = longest_question_bytes(&questions);
    if state.len().saturating_add(longest) > MAX_STATE_AND_QUESTION_BYTES {
        return Err(SystemOneRequestError::OverBudget);
    }
    Ok(json!({"model": model, "state": state, "questions": questions}))
}

/// Refuses an empty, oversized, duplicated or unnamed question set.
///
/// An answer resolves to the name it answers under, so two questions under one name would make one
/// of them unanswerable rather than both, and a name with no answer at all is the unconsumed
/// question this contract refuses to send.
fn validate_questions(questions: &[SystemOneQuestion<'_>]) -> Result<(), SystemOneRequestError> {
    if questions.is_empty() {
        return Err(SystemOneRequestError::EmptyOptions);
    }
    if questions.len() > MAX_QUESTIONS {
        return Err(SystemOneRequestError::TooManyQuestions);
    }
    for (index, question) in questions.iter().enumerate() {
        if question.name.is_empty()
            || question.name.len() > 240
            || !printable(question.name)
            || questions[..index]
                .iter()
                .any(|seen| seen.name == question.name)
        {
            return Err(SystemOneRequestError::InvalidQuestion);
        }
        validate_options(question.options)?;
    }
    Ok(())
}

/// Builds the criteria map, whose keys are exactly the presented option identifiers.
///
/// The value is the caller's description, or the identifier when the caller supplied none.
pub(super) fn criteria(options: &[SystemOneOption]) -> Value {
    let mut map = serde_json::Map::new();
    for option in options {
        let description = if option.description.is_empty() {
            option.id.clone()
        } else {
            option.description.clone()
        };
        map.insert(option.id.clone(), Value::String(description));
    }
    Value::Object(map)
}

/// Length in bytes of the largest single question, which is what shares the budget with the state.
fn longest_question_bytes(questions: &Value) -> usize {
    questions
        .as_object()
        .map(|questions| {
            questions
                .values()
                .map(|question| question.to_string().len())
                .max()
                .unwrap_or_default()
        })
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "systemone_question_set_tests.rs"]
mod tests;
