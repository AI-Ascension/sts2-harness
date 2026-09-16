// SPDX-License-Identifier: MIT

use sts2_harness::game_information::{LookupAgentPort, LookupError};
use sts2_harness::{EpisodeLegalActionSet, PolicyError};

use super::super::RuntimeV3Port;
use super::lookup_agent::FencedLookupAgent;

pub(super) fn run_game_information_lookup(
    port: &mut RuntimeV3Port,
    legal_actions: &EpisodeLegalActionSet,
    agent: &mut dyn LookupAgentPort,
) -> Result<String, PolicyError> {
    let owner = port
        .lookup_policy_owner
        .clone()
        .ok_or(PolicyError::ProviderUnavailable)?;
    let expected = port
        .lookup_policy_binding
        .clone()
        .ok_or(PolicyError::ProviderUnavailable)?;
    let mut session = port
        .lookup_session
        .take()
        .ok_or(PolicyError::ProviderUnavailable)?;
    let mut corpus = port
        .lookup_corpus
        .take()
        .ok_or(PolicyError::ProviderUnavailable)?;
    let mut fenced_agent = FencedLookupAgent {
        owner: owner.clone(),
        expected: expected.clone(),
        agent,
    };
    let result = if port.lookup_replay_mode {
        sts2_harness::game_information::run_lookup_replay_tool_loop(
            &mut session,
            &corpus,
            &mut fenced_agent,
            legal_actions,
            8,
        )
    } else {
        sts2_harness::game_information::run_lookup_tool_loop(
            &mut session,
            &mut corpus,
            port,
            &mut fenced_agent,
            legal_actions,
            8,
        )
    };
    port.lookup_session = Some(session);
    port.lookup_corpus = Some(corpus);
    let action = result.map_err(|error| match error {
        LookupError::Transport => PolicyError::ProviderUnavailable,
        LookupError::Invalid | LookupError::Bounds => PolicyError::ProviderMalformed,
        _ => PolicyError::ProviderUnavailable,
    })?;
    owner
        .lookup_snapshot(Some(&expected))
        .map_err(|_| PolicyError::ProviderUnavailable)?;
    Ok(action)
}

pub(super) fn prepare_game_information_binding(
    port: &mut RuntimeV3Port,
) -> Result<(), sts2_harness::PortError> {
    port.initialize_game_information_binding()
}

pub(super) fn refresh_game_information_binding(
    port: &mut RuntimeV3Port,
    state_id: &str,
    generation: u64,
) -> Result<(), sts2_harness::PortError> {
    RuntimeV3Port::refresh_game_information_binding(port, state_id, generation)
}
