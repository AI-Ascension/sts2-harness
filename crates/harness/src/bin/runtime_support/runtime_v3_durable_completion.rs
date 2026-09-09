// SPDX-License-Identifier: MIT

use sts2_harness::{CompletionRecord, CompletionStatus, EpisodeObservation, EpisodeRunReport};

use super::super::worker_store::{snapshot, try_lock, try_lock_recovery};
use super::super::workflow_binding::completion_stage_allowed;
use super::DurableHandle;
use super::support::sha256_json;

impl DurableHandle {
    /// Reports whether this handle's episode already has a durable successful completion.
    /// Cleanup failures must consult this boundary before attempting interrupted-unknown
    /// quarantine, because a completion committed before shutdown is authoritative.
    pub(in super::super) fn is_completed(&self) -> Result<bool, String> {
        try_lock_recovery(&self.store)?
            .completion(&self.lineage.episode_id)
            .map(|completion| {
                completion
                    .is_some_and(|completion| completion.status == CompletionStatus::Completed)
            })
            .map_err(|error| format!("cannot inspect runtime-v3 completion state: {error}"))
    }

    pub(in super::super) fn complete_episode(
        &self,
        report: &EpisodeRunReport,
    ) -> Result<(), String> {
        self.complete_observation(report.final_observation())
    }

    pub(in super::super) fn complete_observation(
        &self,
        observation: &EpisodeObservation,
    ) -> Result<(), String> {
        self.complete_observation_with_mode(observation, false)
    }

    pub(in super::super) fn complete_combat_observation(
        &self,
        observation: &EpisodeObservation,
    ) -> Result<(), String> {
        if !self.workflow_binding.is_combat_demo() {
            return Err(String::from(
                "runtime-v3 combat completion requires a combat-demo workflow binding",
            ));
        }
        self.complete_observation_with_mode(observation, true)
    }

    fn complete_observation_with_mode(
        &self,
        observation: &EpisodeObservation,
        combat_completion: bool,
    ) -> Result<(), String> {
        if !completion_stage_allowed(observation.stage(), combat_completion) {
            return Err(String::from(
                "runtime-v3 completion requires a terminal observation",
            ));
        }
        let observation_bytes = serde_json::to_vec(observation.fair_play().as_value())
            .map_err(|error| format!("cannot encode runtime-v3 terminal observation: {error}"))?;
        let expected_next = self
            .next_checkpoint
            .try_borrow()
            .map_err(|_| String::from("runtime-v3 checkpoint sequence is already borrowed"))?;
        let mut store = try_lock(&self.store)?;
        let snapshot = snapshot(&store, &self.lineage, &self.fingerprint)?;
        let checkpoint = snapshot
            .episode
            .last_checkpoint
            .ok_or_else(|| String::from("runtime-v3 terminal observation was not checkpointed"))?;
        if checkpoint.sequence.checked_add(1) != Some(*expected_next)
            || checkpoint.lineage != self.lineage
            || checkpoint.fingerprint != self.fingerprint
            || checkpoint.state_id != observation.state_id()
            || checkpoint.generation != observation.generation()
            || checkpoint.observation != observation_bytes
        {
            return Err(String::from(
                "runtime-v3 terminal observation does not match the latest checkpoint",
            ));
        }
        let checkpoint_sequence = checkpoint.sequence;
        let terminal_ref = format!(
            "terminal-{}-{}",
            super::super::super::runtime_v3_wire::stage_name(observation.stage()),
            observation.generation()
        );
        let result_digest = sha256_json(observation.fair_play().as_value())?;
        let completion = CompletionRecord::new(
            self.lineage.clone(),
            CompletionStatus::Completed,
            terminal_ref,
            checkpoint_sequence,
            result_digest,
        )
        .map_err(|error| format!("runtime-v3 completion is invalid: {error}"))?;
        store
            .record_completion(&completion)
            .map(|_| ())
            .map_err(|error| format!("cannot persist runtime-v3 completion: {error}"))
    }
}
