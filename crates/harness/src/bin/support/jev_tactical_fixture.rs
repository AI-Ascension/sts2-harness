// SPDX-License-Identifier: MIT

//! Original synthetic numerical fixtures. No game assets, observed trajectories, or provider calls.

use serde_json::{Map, Value, json};

pub(super) fn body(count: usize) -> Value {
    let criteria: Map<String, Value> = (0..count)
        .map(|index| (format!("action-{index:02}"), json!("Synthetic candidate")))
        .collect();
    json!({
        "model": "jev-latest", "state": "Synthetic admitted state",
        "questions": {"action": {
            "type": "choice", "instructions": "Preserve health and complete the run",
            "criteria": criteria,
        }},
    })
}

pub(super) fn reply(body: &Value, winner: &str) -> Value {
    let mut answers = Map::new();
    let questions = body["questions"].as_object().expect("fixture questions");
    for (key, question) in questions {
        let good = question["instructions"]["candidate_id"].as_str() == Some(winner);
        let answer = match question["type"].as_str() {
            Some("choice") => {
                let criteria = question["criteria"].as_object().expect("fixture criteria");
                let first = criteria.keys().next().expect("fixture candidate");
                let probabilities: Map<String, Value> = criteria
                    .keys()
                    .map(|id| (id.clone(), json!(if id == first { 1.0 } else { 0.0 })))
                    .collect();
                json!({"type": "choice", "choice": first, "confidence": 0.9, "probabilities": probabilities})
            }
            Some("score") => {
                let (score, low, high) = if good {
                    (1.8, 0.1, 0.9)
                } else {
                    (0.2, 0.9, 0.1)
                };
                json!({
                    "type": "score", "score": score, "confidence": 0.9,
                    "legend": {"0": question["criteria"][0], "1": question["criteria"][1], "2": question["criteria"][2]},
                    "probabilities": {"0": low, "1": 0.0, "2": high},
                })
            }
            Some("noul") => json!({"type": "noul", "noul": 0.95}),
            _ => Value::Null,
        };
        answers.insert(key.clone(), answer);
    }
    json!({"model": "jev-1.13.0", "answers": answers, "usage": {"input_tokens": 100, "output_tokens": 100}})
}
