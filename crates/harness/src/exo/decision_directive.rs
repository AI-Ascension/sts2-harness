// SPDX-License-Identifier: MIT

//! Parsing for the decisions that direct observation rather than act.
//!
//! `wait` and `reobserve` both decline to dispatch anything. They are parsed together because what
//! separates them from an action is the same in both cases: no action identifier, no confidence in
//! one, and no recovery operation. The one asymmetry is the candidate an abstention may carry.

use super::{Decision, DecisionError, valid_id};

pub(super) fn parse_observation_directive(
    object: &serde_json::Map<String, serde_json::Value>,
    decision: &str,
    rationale: String,
) -> Result<Decision, DecisionError> {
    if object.keys().any(|key| {
        matches!(
            key.as_str(),
            "action_id" | "action_ids" | "confidence" | "recovery_kind" | "operation_id"
        )
    }) {
        return Err(DecisionError::UnknownField);
    }
    // A candidate belongs to an abstention and nothing else. Waiting is not declining to choose
    // between options, so a candidate on a wait is malformed rather than ignored.
    let carries_candidate =
        object.contains_key("candidate_action_id") || object.contains_key("candidate_confidence");
    if decision != "reobserve" && carries_candidate {
        return Err(DecisionError::UnknownField);
    }
    match decision {
        "wait" => Ok(Decision::Wait { rationale }),
        "reobserve" => {
            let candidate_action_id = object
                .get("candidate_action_id")
                .map(|value| {
                    value
                        .as_str()
                        .filter(|value| valid_id(value))
                        .map(str::to_owned)
                        .ok_or(DecisionError::InvalidValue)
                })
                .transpose()?;
            let candidate_confidence = object
                .get("candidate_confidence")
                .map(|value| {
                    value
                        .as_u64()
                        .and_then(|value| u8::try_from(value).ok())
                        .filter(|value| *value <= 100)
                        .ok_or(DecisionError::InvalidValue)
                })
                .transpose()?;
            // A confidence with nothing to be confident about states nothing.
            if candidate_confidence.is_some() && candidate_action_id.is_none() {
                return Err(DecisionError::MissingField);
            }
            Ok(Decision::Reobserve {
                rationale,
                candidate_action_id,
                candidate_confidence,
            })
        }
        _ => Err(DecisionError::InvalidValue),
    }
}

#[cfg(test)]
mod tests {
    use super::super::Decision;
    use super::super::parse_decision;

    fn parse(body: &str) -> Result<Decision, super::DecisionError> {
        parse_decision(body.as_bytes())
    }

    #[test]
    fn an_abstention_may_name_what_it_would_have_taken() {
        assert_eq!(
            parse(
                r#"{"decision":"reobserve","rationale":"below the gate",
                    "candidate_action_id":"proceed:60","candidate_confidence":6}"#
            ),
            Ok(Decision::Reobserve {
                rationale: String::from("below the gate"),
                candidate_action_id: Some(String::from("proceed:60")),
                candidate_confidence: Some(6),
            })
        );
    }

    #[test]
    fn an_abstention_without_a_candidate_is_unchanged() {
        assert_eq!(
            parse(r#"{"decision":"reobserve","rationale":"nothing to say"}"#),
            Ok(Decision::Reobserve {
                rationale: String::from("nothing to say"),
                candidate_action_id: None,
                candidate_confidence: None,
            })
        );
    }

    #[test]
    fn a_candidate_belongs_to_an_abstention_and_nothing_else() {
        // Waiting is not declining to choose between options, so a candidate there says nothing.
        assert!(
            parse(r#"{"decision":"wait","rationale":"r","candidate_action_id":"proceed:60"}"#)
                .is_err()
        );
    }

    #[test]
    fn a_confidence_with_nothing_to_be_confident_about_is_refused() {
        assert!(
            parse(r#"{"decision":"reobserve","rationale":"r","candidate_confidence":40}"#).is_err()
        );
    }

    #[test]
    fn a_candidate_is_still_held_to_the_shapes_every_identifier_is() {
        for body in [
            r#"{"decision":"reobserve","rationale":"r","candidate_action_id":""}"#,
            r#"{"decision":"reobserve","rationale":"r","candidate_action_id":7}"#,
            r#"{"decision":"reobserve","rationale":"r","candidate_action_id":"a","candidate_confidence":101}"#,
            r#"{"decision":"reobserve","rationale":"r","candidate_action_id":"a","candidate_confidence":-1}"#,
        ] {
            assert!(parse(body).is_err(), "{body} must be refused");
        }
    }

    #[test]
    fn an_action_may_not_carry_a_candidate_because_it_already_carries_its_choice() {
        assert!(
            parse(
                r#"{"decision":"action","action_id":"play:1","rationale":"r",
                    "candidate_action_id":"proceed:2"}"#
            )
            .is_err()
        );
    }
}
