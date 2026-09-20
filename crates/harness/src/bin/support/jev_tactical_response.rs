// SPDX-License-Identifier: MIT

//! Checks the entire answer set before any ranking; malformed evidence is never a partial success.

use super::{ACTION_QUESTION, AXES, Error, key};
use serde::Serialize;
use serde_json::{Map, Value};

const TOLERANCE: f64 = 0.000_01;

#[derive(Clone, Debug, Serialize)]
pub(super) struct Row {
    pub action_id: String,
    pub scores: [f64; 6],
    pub min_confidence: f64,
    pub evidence: f64,
    pub utility: f64,
}

pub(super) fn rows(body: &Value, response: &Value) -> Result<Vec<Row>, Error> {
    let questions = body["questions"].as_object().ok_or(Error::Request)?;
    let answers = response["answers"].as_object().ok_or(Error::Answers)?;
    if questions.len() != answers.len()
        || !questions.keys().all(|id| answers.contains_key(id))
        || response["model"].as_str().is_none_or(str::is_empty)
    {
        return Err(Error::Answers);
    }
    let choices = questions.get(ACTION_QUESTION).ok_or(Error::Request)?["criteria"]
        .as_object()
        .ok_or(Error::Request)?;
    if !(2..=super::MAX_OPTIONS).contains(&choices.len())
        || questions.len() != 1 + choices.len() * 7
    {
        return Err(Error::Request);
    }
    validate_choice(&answers[ACTION_QUESTION], choices)?;
    choices
        .keys()
        .enumerate()
        .map(|(index, id)| row(index, id, questions, answers))
        .collect()
}

fn row(
    index: usize,
    id: &str,
    questions: &Map<String, Value>,
    answers: &Map<String, Value>,
) -> Result<Row, Error> {
    let mut scores = [0.0; 6];
    let mut confidence: f64 = 1.0;
    let mut utility = 0.0;
    let mut weight_sum = 0.0;
    for (column, (axis, _, weight)) in AXES.iter().enumerate() {
        let id = key(index, axis);
        let answer = answers.get(&id).ok_or(Error::Answers)?;
        let criteria = questions.get(&id).ok_or(Error::Request)?["criteria"]
            .as_array()
            .ok_or(Error::Request)?;
        scores[column] = validate_score(answer, criteria)? / 2.0;
        confidence = confidence.min(unit(&answer["confidence"])?);
        utility += scores[column] * weight;
        weight_sum += weight;
    }
    let evidence = answers.get(&key(index, "evidence")).ok_or(Error::Answers)?;
    if evidence["type"].as_str() != Some("noul") {
        return Err(Error::Answers);
    }
    Ok(Row {
        action_id: id.to_owned(),
        scores,
        min_confidence: confidence,
        evidence: unit(&evidence["noul"])?,
        utility: utility / weight_sum,
    })
}

fn validate_choice(answer: &Value, criteria: &Map<String, Value>) -> Result<(), Error> {
    if answer["type"].as_str() != Some("choice") {
        return Err(Error::Answers);
    }
    let choice = answer["choice"].as_str().ok_or(Error::Answers)?;
    let probabilities = distribution(answer, criteria.keys().map(String::as_str))?;
    let chosen = unit(probabilities.get(choice).ok_or(Error::Distribution)?)?;
    for probability in probabilities.values() {
        if unit(probability)? > chosen + TOLERANCE {
            return Err(Error::Distribution);
        }
    }
    unit(&answer["confidence"])?;
    Ok(())
}

fn validate_score(answer: &Value, criteria: &[Value]) -> Result<f64, Error> {
    if answer["type"].as_str() != Some("score") || criteria.len() != 3 {
        return Err(Error::Answers);
    }
    let probabilities = distribution(answer, ["0", "1", "2"])?;
    let legend = answer["legend"].as_object().ok_or(Error::Answers)?;
    if legend.len() != 3
        || (0..3).any(|index| legend.get(&index.to_string()) != criteria.get(index))
    {
        return Err(Error::Answers);
    }
    let expected = unit(&probabilities["1"])? + 2.0 * unit(&probabilities["2"])?;
    let score = answer["score"]
        .as_f64()
        .filter(|score| score.is_finite())
        .ok_or(Error::Distribution)?;
    if !(0.0..=2.0).contains(&score) || (score - expected).abs() > TOLERANCE {
        return Err(Error::Distribution);
    }
    unit(&answer["confidence"])?;
    Ok(score)
}

fn distribution<'a, 'b>(
    answer: &'a Value,
    keys: impl IntoIterator<Item = &'b str>,
) -> Result<&'a Map<String, Value>, Error> {
    let probabilities = answer["probabilities"]
        .as_object()
        .ok_or(Error::Distribution)?;
    let mut sum = 0.0;
    let mut count = 0;
    for key in keys {
        sum += unit(probabilities.get(key).ok_or(Error::Distribution)?)?;
        count += 1;
    }
    if count != probabilities.len() || (sum - 1.0).abs() > TOLERANCE {
        return Err(Error::Distribution);
    }
    Ok(probabilities)
}

pub(super) fn unit(value: &Value) -> Result<f64, Error> {
    value
        .as_f64()
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        .ok_or(Error::Distribution)
}
