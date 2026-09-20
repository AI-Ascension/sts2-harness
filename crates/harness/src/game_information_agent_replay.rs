// SPDX-License-Identifier: MIT
//! Replays retained tool transcripts; a replay never performs a live read.

use super::*;

/// Replays the next archived tool transcript without an MCP callback.
/// Every query must match the retained request byte-for-byte except its transport correlation.
pub fn run_lookup_replay_tool_loop<A: LookupAgentPort>(
    session: &mut LookupSession,
    corpus: &MemoryCorpus,
    agent: &mut A,
    legal_actions: &EpisodeLegalActionSet,
    max_turns: usize,
) -> Result<String, LookupError> {
    if max_turns == 0 || max_turns > 32 {
        return Err(LookupError::Bounds);
    }
    if session.binding.snapshot.as_ref().is_some_and(|snapshot| {
        snapshot["state_generation"].as_u64() != Some(legal_actions.generation())
    }) {
        return Err(LookupError::Reobserve);
    }
    let mut feedback = LookupFeedback::Start;
    for remaining in (0..max_turns).rev() {
        if session.binding.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot["state_generation"].as_u64() != Some(legal_actions.generation())
        }) {
            return Err(LookupError::Reobserve);
        }
        let turn = agent.next_turn(LookupAgentInput {
            binding: &session.binding,
            legal_actions,
            feedback: &feedback,
            remaining_turns: remaining,
            optional_byte_budget: session.policy.optional_byte_budget,
        })?;
        match turn {
            LookupTurn::Decide { action_id } => {
                return legal_actions
                    .actions()
                    .iter()
                    .any(|action| action.action_id() == action_id)
                    .then_some(action_id)
                    .ok_or(LookupError::Invalid);
            }
            LookupTurn::Query {
                operation_id,
                request,
            } => {
                let record_ordinal = session.replay_cursor;
                let record = session
                    .records
                    .get(record_ordinal)
                    .cloned()
                    .ok_or(LookupError::Divergence)?;
                let request = validation::decode_strict(&request)?;
                validation::validate_request(&request)?;
                if record.operation_id != operation_id
                    || without_correlation(&record.request) != without_correlation(&request)
                {
                    return Err(LookupError::Divergence);
                }
                if matches!(record.error, Some(LookupError::Reobserve)) {
                    return Err(LookupError::Reobserve);
                }
                session.replay_cursor = session
                    .replay_cursor
                    .checked_add(1)
                    .ok_or(LookupError::Bounds)?;
                feedback = match session.replay(&record, &record.request, corpus) {
                    Ok(delivery) => LookupFeedback::Data {
                        record_ordinal,
                        delivery: Box::new(delivery),
                    },
                    Err(LookupError::Divergence | LookupError::Scope | LookupError::Reobserve) => {
                        return Err(LookupError::Divergence);
                    }
                    Err(error) => LookupFeedback::Error(error),
                };
            }
            LookupTurn::Bootstrap {
                operation_id,
                request,
            } => {
                let record_ordinal = session.bootstrap_replay_cursor;
                let record = session
                    .bootstrap_records()
                    .get(record_ordinal)
                    .ok_or(LookupError::Divergence)?;
                let request = validation::decode_strict(&request)?;
                if record.operation_id != operation_id
                    || without_correlation(&record.request) != without_correlation(&request)
                {
                    return Err(LookupError::Divergence);
                }
                let response = record
                    .response
                    .clone()
                    .ok_or_else(|| record.error.clone().unwrap_or(LookupError::Divergence))?;
                let response_scope = response["scope"]
                    .as_object()
                    .ok_or(LookupError::Divergence)?;
                if response_scope["run_id"] != session.binding.scope.run_id
                    || response_scope["authority_epoch"] != session.binding.authority_epoch
                    || response_scope["content_manifest_id"] != session.binding.content_manifest_id
                    || response_scope["locale"] != session.binding.locale
                {
                    return Err(LookupError::Divergence);
                }
                let mut validated_request = request.clone();
                validated_request["scope"] = response["scope"].clone();
                validated_request["correlation_id"] = response["correlation_id"].clone();
                let snapshot =
                    crate::game_information_binding::game_information_bootstrap::select_snapshot(
                        &validated_request,
                        &response,
                    )
                    .map_err(|_| LookupError::Divergence)?;
                if record.binding.snapshot.as_ref() != Some(&snapshot) {
                    return Err(LookupError::Divergence);
                }
                session.bootstrap_replay_cursor = session
                    .bootstrap_replay_cursor
                    .checked_add(1)
                    .ok_or(LookupError::Bounds)?;
                feedback = LookupFeedback::Bootstrap {
                    record_ordinal,
                    response,
                };
            }
            LookupTurn::ReadRetained {
                record_ordinal,
                offset,
            } => {
                let record = session
                    .records
                    .get(record_ordinal)
                    .ok_or(LookupError::MissingRetention)?;
                let bytes = session.read_retained(record, corpus, offset)?;
                feedback = LookupFeedback::Bytes {
                    record_ordinal,
                    offset,
                    total_bytes: record.source_bytes,
                    bytes,
                };
            }
            // The archive retains MCP tool transcripts, not history: a history turn here has no
            // retained answer to replay, so replaying one would either invent an answer or read
            // live. It is a divergence rather than a fresh read presented as a replay.
            LookupTurn::History { .. } => return Err(LookupError::Divergence),
        }
    }
    Err(LookupError::Bounds)
}

fn without_correlation(value: &Value) -> Value {
    let mut request = value.clone();
    if let Some(object) = request.as_object_mut() {
        object.remove("correlation_id");
    }
    request
}
