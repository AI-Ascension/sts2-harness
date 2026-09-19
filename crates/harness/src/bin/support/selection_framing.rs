// SPDX-License-Identifier: MIT

//! What the choice question says about the decision, beyond the options themselves.

use serde_json::Value;

/// Composes the framing the choice question carries, which is the objective and, at a screen that
/// offers a set to choose from, what choosing from it means.
///
/// A card taken at a reward is kept for the rest of the run, while every other decision on the
/// board is spent on the turn it is made. Nothing in the state says so, and the model was reading
/// the two kinds of decision the same way: this says which kind it is looking at.
///
/// It is framing, not description. It states what the decision does, so it belongs beside the
/// objective — which is also the operator speaking — rather than in an option's description, where
/// every word is a value the host supplied.
pub(super) fn framing(objective: &str, observation: &Value) -> String {
    let offers_a_set = observation
        .get("state")
        .and_then(|state| state.get("choices"))
        .and_then(Value::as_array)
        .is_some_and(|choices| !choices.is_empty());
    if !offers_a_set {
        return objective.to_owned();
    }
    let note = "A card chosen here is kept for the rest of the run and will be drawn on later                 turns, so it is a lasting choice rather than a play made this turn.";
    if objective.is_empty() {
        return String::from(note);
    }
    format!("{objective}. {note}")
}

#[cfg(test)]
mod tests {
    use super::framing;
    use serde_json::json;

    #[test]
    fn a_selection_screen_says_the_choice_is_kept() {
        let observation = json!({"state": {"state": "selection", "choices": ["card:1:Strike"]}});
        let text = framing("win", &observation);
        assert!(text.starts_with("win. "), "{text}");
        assert!(text.contains("kept for the rest of the run"), "{text}");
    }

    #[test]
    fn every_other_screen_carries_the_objective_alone() {
        // A play made this turn is spent on this turn, so there is nothing extra to say about it.
        for observation in [
            json!({"state": {"state": "combat", "turn_index": 1}}),
            json!({"state": {"state": "selection", "choices": []}}),
            json!({}),
        ] {
            assert_eq!(framing("win", &observation), "win");
        }
    }

    #[test]
    fn an_absent_objective_leaves_the_note_standing_alone() {
        let observation = json!({"state": {"state": "selection", "choices": ["card:1:Strike"]}});
        let text = framing("", &observation);
        assert!(text.starts_with("A card chosen here"), "{text}");
    }
}
