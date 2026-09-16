// SPDX-License-Identifier: MIT

use std::time::Duration;

use serde_json::json;
use sts2_harness::{
    Decision, DecisionInput, DecisionSource, EpisodeRuntimePort, ExoProcessConfig,
    ModelExecutionId, PolicyError, exo_lookup_process::ExoLookupProcess,
};

/// Runtime decision source that uses the bounded duplex lookup protocol and
/// leaves lookup admission and MCP delivery with the existing episode port.
pub(super) struct LookupAgentDecisionSource {
    process: ExoProcessConfig,
    revision: String,
    timeout: Duration,
    execution_id: Option<ModelExecutionId>,
}

impl LookupAgentDecisionSource {
    pub(super) fn new(process: ExoProcessConfig, revision: String, timeout: Duration) -> Self {
        Self {
            process,
            revision,
            timeout,
            execution_id: None,
        }
    }
}

impl DecisionSource for LookupAgentDecisionSource {
    fn decide(&mut self, _input: &DecisionInput) -> Result<Decision, PolicyError> {
        Err(PolicyError::ProviderUnavailable)
    }

    fn decide_with_game_information(
        &mut self,
        input: &DecisionInput,
        runtime: &mut dyn EpisodeRuntimePort,
    ) -> Result<Decision, PolicyError> {
        input
            .observation
            .assert_actionable()
            .map_err(|_| PolicyError::InputBlocked)?;
        input
            .legal_actions
            .assert_matches(input.observation.state_id(), input.observation.generation())
            .map_err(|_| PolicyError::StaleCatalog)?;
        let legal_action_ids = input
            .legal_actions
            .actions()
            .iter()
            .map(|action| action.action_id().to_owned())
            .collect::<Vec<_>>();
        let request_id = format!(
            "lookup-request-{}",
            sts2_harness::sha256_hex(format!(
                "{}:{}:{}",
                input.execution_id.get(),
                input.observation.state_id(),
                input.observation.generation()
            ))
        );
        let turn_id = format!("lookup-turn-{}", input.execution_id.get());
        let request = json!({
            "schema": "sts2.exo-decision-v1",
            "provider_revision": &self.revision,
            "model_execution_id": format!("execution-{}", input.execution_id.get()),
            "state_id": input.observation.state_id(),
            "generation": input.observation.generation(),
            "observation": input.observation.fair_play().as_value(),
            "legal_action_ids": legal_action_ids,
            "objective": &input.objective,
            "hard_constraints": &input.hard_constraints,
            "max_response_bytes": 8192,
        });
        let mut agent = ExoLookupProcess::new(
            self.process.clone(),
            request_id,
            turn_id,
            request,
            self.timeout,
        )
        .map_err(map_lookup_error)?;
        let action_id = runtime.run_game_information_lookup(&input.legal_actions, &mut agent)?;
        self.execution_id = Some(input.execution_id);
        Ok(Decision::Action {
            action_id,
            rationale: String::from("bounded selected-policy game-information decision"),
            confidence: None,
        })
    }

    fn model_execution_id(&self) -> Option<ModelExecutionId> {
        self.execution_id
    }
}

fn map_lookup_error(error: sts2_harness::game_information::LookupError) -> PolicyError {
    use sts2_harness::game_information::LookupError;
    match error {
        LookupError::Transport => PolicyError::ProviderUnavailable,
        LookupError::Invalid | LookupError::Bounds => PolicyError::ProviderMalformed,
        _ => PolicyError::ProviderUnavailable,
    }
}
