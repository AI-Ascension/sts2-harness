// SPDX-License-Identifier: MIT
//! Wire-protocol turn adapter for the owned lookup subprocess.
//!
//! Profile selection is enforced here: a frame carrying a turn the selected profile did not
//! admit is refused rather than answered, so a provider cannot widen its own surface.

use super::*;

impl LookupAgentPort for ExoLookupProcess {
    fn next_turn(&mut self, input: LookupAgentInput<'_>) -> Result<LookupTurn, LookupError> {
        let result = (|| {
            if self
                .binding
                .as_ref()
                .is_some_and(|binding| !binding.same_owner(input.binding))
                || self
                    .byte_budget
                    .is_some_and(|budget| budget != input.optional_byte_budget)
                || self.request["generation"].as_u64() != Some(input.legal_actions.generation())
                || self.request["state_id"].as_str() != Some(input.legal_actions.state_id())
                || self.request["legal_action_ids"]
                    != serde_json::json!(
                        input
                            .legal_actions
                            .actions()
                            .iter()
                            .map(|a| a.action_id())
                            .collect::<Vec<_>>()
                    )
            {
                return Err(LookupError::Scope);
            }
            let payload = if self.sequence == 0 {
                if *input.feedback != LookupFeedback::Start {
                    return Err(LookupError::Scope);
                }
                self.binding = Some(input.binding.clone());
                self.byte_budget = Some(input.optional_byte_budget);
                ExoLookupPayload::Start {
                    request: self.request.clone(),
                    optional_byte_budget: input.optional_byte_budget,
                }
            } else {
                ExoLookupPayload::Feedback {
                    value: crate::exo_lookup_wire::feedback_value(
                        input.feedback,
                        input.optional_byte_budget,
                    )?,
                }
            };
            match self.exchange(payload)? {
                ExoLookupPayload::Query { arguments } => {
                    crate::exo_lookup_wire::query_turn(arguments, &input)
                }
                ExoLookupPayload::Bootstrap { arguments } => {
                    // The history profile is additive over the bootstrap one, so selecting it
                    // widens the surface instead of trading the shipped capability away.
                    if !matches!(
                        self.profile,
                        ExoLookupProfile::Bootstrap | ExoLookupProfile::History
                    ) {
                        return Err(LookupError::Invalid);
                    }
                    crate::exo_lookup_wire::bootstrap_turn(arguments)
                }
                ExoLookupPayload::History { arguments } => {
                    if self.profile != ExoLookupProfile::History {
                        return Err(LookupError::Invalid);
                    }
                    crate::game_information::history::turn(arguments)
                }
                ExoLookupPayload::ReadRetained {
                    record_ordinal,
                    offset,
                } if record_ordinal < 256 && offset <= 65_536 => Ok(LookupTurn::ReadRetained {
                    record_ordinal,
                    offset,
                }),
                ExoLookupPayload::Decision { action_id }
                    if input
                        .legal_actions
                        .actions()
                        .iter()
                        .any(|a| a.action_id() == action_id) =>
                {
                    self.closed = true;
                    self.sender.take();
                    Ok(LookupTurn::Decide { action_id })
                }
                _ => Err(LookupError::Invalid),
            }
        })();
        if result.is_err() {
            self.closed = true;
            self.sender.take();
        }
        result
    }
}
