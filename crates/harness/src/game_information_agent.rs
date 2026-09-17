// SPDX-License-Identifier: MIT
use super::*;
use crate::EpisodeLegalActionSet;
include!("game_information_agent_bootstrap.rs");

/// Additive harness agent contract. Existing closed Exo terminal-decision profiles are unchanged.
#[derive(Clone, Debug)]
pub enum LookupTurn {
    Query {
        operation_id: String,
        request: Vec<u8>,
    },
    Bootstrap {
        operation_id: String,
        request: Vec<u8>,
    },
    ReadRetained {
        record_ordinal: usize,
        offset: usize,
    },
    Decide {
        action_id: String,
    },
}

/// Tool feedback is a separate data channel; it never supplies instructions or legal actions.
#[derive(Clone, Debug, PartialEq)]
pub enum LookupFeedback {
    Start,
    Data {
        record_ordinal: usize,
        delivery: Box<LookupDelivery>,
    },
    Bootstrap {
        record_ordinal: usize,
        response: Value,
    },
    Bytes {
        record_ordinal: usize,
        offset: usize,
        total_bytes: usize,
        bytes: Vec<u8>,
    },
    Error(LookupError),
}

pub struct LookupAgentInput<'a> {
    pub binding: &'a LookupBinding,
    pub legal_actions: &'a EpisodeLegalActionSet,
    pub feedback: &'a LookupFeedback,
    pub remaining_turns: usize,
    /// Serialized optional prepared-data budget; transport envelopes have separate bounds.
    pub optional_byte_budget: usize,
}

/// The provider owner maps its explicitly admitted model/tool protocol to these bounded turns.
pub trait LookupAgentPort {
    fn next_turn(&mut self, input: LookupAgentInput<'_>) -> Result<LookupTurn, LookupError>;
}

/// Runs at most 32 read/decision turns, returning only an ID in the unchanged host legal set.
/// It has no gameplay dispatch authority; normal episode admission/settlement remains necessary.
pub fn run_lookup_tool_loop<A: LookupAgentPort, M: LookupMcpPort>(
    session: &mut LookupSession,
    corpus: &mut MemoryCorpus,
    mcp: &mut M,
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
        let turn = agent.next_turn(LookupAgentInput {
            binding: &session.binding,
            legal_actions,
            feedback: &feedback,
            remaining_turns: remaining,
            optional_byte_budget: session.policy.optional_byte_budget,
        })?;
        if let LookupTurn::Decide { action_id } = turn {
            return legal_actions
                .actions()
                .iter()
                .any(|action| action.action_id() == action_id)
                .then_some(action_id)
                .ok_or(LookupError::Invalid);
        }
        let binding = session.binding.clone();
        feedback = handle_read(session, corpus, mcp, &binding, turn);
    }
    Err(LookupError::Bounds)
}

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
                let mut validated_request = request.clone();
                validated_request["scope"] = response["scope"].clone();
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

fn handle_read<M: LookupMcpPort>(
    session: &mut LookupSession,
    corpus: &mut MemoryCorpus,
    mcp: &mut M,
    binding: &LookupBinding,
    turn: LookupTurn,
) -> LookupFeedback {
    match turn {
        LookupTurn::Query {
            operation_id,
            request,
        } => match session.query_port(binding, &operation_id, &request, corpus, mcp) {
            Ok(delivery) => LookupFeedback::Data {
                record_ordinal: session.records.len() - 1,
                delivery: Box::new(delivery),
            },
            Err(error) => LookupFeedback::Error(error),
        },
        LookupTurn::Bootstrap {
            operation_id,
            request,
        } => handle_bootstrap(session, mcp, operation_id, request),
        LookupTurn::ReadRetained {
            record_ordinal,
            offset,
        } => {
            let Some(record) = session.records.get(record_ordinal) else {
                return LookupFeedback::Error(LookupError::MissingRetention);
            };
            match session.read_retained(record, corpus, offset) {
                Ok(bytes) => LookupFeedback::Bytes {
                    record_ordinal,
                    offset,
                    total_bytes: record.source_bytes,
                    bytes,
                },
                Err(error) => LookupFeedback::Error(error),
            }
        }
        LookupTurn::Decide { .. } => LookupFeedback::Error(LookupError::Invalid),
    }
}
