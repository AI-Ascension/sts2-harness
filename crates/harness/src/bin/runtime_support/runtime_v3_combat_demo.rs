// SPDX-License-Identifier: MIT

use super::RuntimeV3Port;
#[path = "runtime_v3_combat_replay.rs"]
mod replay;
use super::super::runtime_v3_telemetry::CleanupStatus;
use serde_json::json;
use std::time::{Duration, Instant};
use sts2_harness::{
    ActionIdentity, Decision, DecisionInput, DecisionSource, DispatchStatus, EpisodeObservation,
    EpisodeRunnerConfig, EpisodeRuntimePort, EpisodeShutdown, EpisodeStage, ModelExecutionId,
    RecoveryPort, TransitionReceipt, verify_settlement,
};

pub(super) struct CombatDemoReport {
    terminal_observation: EpisodeObservation,
    steps: u32,
}

impl CombatDemoReport {
    pub(super) fn terminal_observation(&self) -> &EpisodeObservation {
        &self.terminal_observation
    }

    pub(super) const fn steps(&self) -> u32 {
        self.steps
    }

    pub(super) fn terminal_observation_digest(&self) -> String {
        replay::Replay::observation_digest(&self.terminal_observation)
    }
}

pub(super) struct CombatDemoFailure {
    message: String,
    terminal_observation: Option<EpisodeObservation>,
    cleanup_status: CleanupStatus,
}

impl CombatDemoFailure {
    pub(super) fn message(&self) -> &str {
        &self.message
    }

    pub(super) fn terminal_observation(&self) -> Option<&EpisodeObservation> {
        self.terminal_observation.as_ref()
    }

    pub(super) const fn cleanup_status(&self) -> CleanupStatus {
        self.cleanup_status
    }
}

pub(super) fn run<S: DecisionSource>(
    port: &mut RuntimeV3Port,
    source: &mut S,
    config: &EpisodeRunnerConfig,
) -> Result<CombatDemoReport, CombatDemoFailure> {
    if let Err(error) = port.launch() {
        return Err(CombatDemoFailure {
            message: error.to_string(),
            terminal_observation: None,
            cleanup_status: CleanupStatus::Failed,
        });
    }
    let result = run_inner(port, source, config);
    let cleanup = EpisodeShutdown
        .close(port)
        .map_err(|error| error.to_string());
    match (result, cleanup) {
        (Ok(report), Ok(())) => Ok(report),
        (Ok(report), Err(error)) => Err(CombatDemoFailure {
            message: format!("combat demo cleanup failed: {error}"),
            terminal_observation: Some(report.terminal_observation),
            cleanup_status: CleanupStatus::Failed,
        }),
        (Err(error), Ok(())) => Err(CombatDemoFailure {
            message: error,
            terminal_observation: None,
            cleanup_status: CleanupStatus::Clean,
        }),
        (Err(error), Err(cleanup_error)) => Err(CombatDemoFailure {
            message: format!("{error}; combat demo cleanup failed: {cleanup_error}"),
            terminal_observation: None,
            cleanup_status: CleanupStatus::Failed,
        }),
    }
}

fn run_inner<S: DecisionSource>(
    port: &mut RuntimeV3Port,
    source: &mut S,
    config: &EpisodeRunnerConfig,
) -> Result<CombatDemoReport, String> {
    let deadline = Instant::now() + Duration::from_secs(900);
    let mut steps = 0_u32;
    let mut saw_combat = false;
    let replay = replay::Replay::load()?;
    while Instant::now() < deadline && steps < config.max_steps() {
        let before = port.observe().map_err(|error| error.to_string())?;
        saw_combat |= before.stage() == EpisodeStage::Combat;
        if saw_combat
            && matches!(
                before.stage(),
                EpisodeStage::Reward | EpisodeStage::Defeat | EpisodeStage::Victory
            )
        {
            replay.finish(steps, &before)?;
            return Ok(CombatDemoReport {
                terminal_observation: before,
                steps,
            });
        }
        if before.stage() != EpisodeStage::Combat || !before.input_enabled() {
            std::thread::sleep(Duration::from_millis(250));
            continue;
        }
        if execute_step(port, source, config, &replay, steps, &before)? {
            steps += 1;
        }
    }
    Err(String::from("combat demo reached its time or action bound"))
}

fn execute_step<S: DecisionSource>(
    port: &mut RuntimeV3Port,
    source: &mut S,
    config: &EpisodeRunnerConfig,
    replay: &replay::Replay,
    steps: u32,
    before: &EpisodeObservation,
) -> Result<bool, String> {
    let actions = port
        .legal_actions(before.state_id(), before.generation())
        .map_err(|error| error.to_string())?;
    let input = DecisionInput::new(
        ModelExecutionId::new(u64::from(steps) + 1)
            .ok_or_else(|| String::from("model execution identity exhausted"))?,
        before.clone(),
        actions.clone(),
        config.objective(),
        config.hard_constraints().to_vec(),
    );
    let decision = match replay.decide(steps, before, &actions)? {
        Some(decision) => decision,
        None => source.decide(&input).map_err(|error| error.to_string())?,
    };
    let current = port.observe().map_err(|error| error.to_string())?;
    if current.generation() != before.generation() || current.state_id() != before.state_id() {
        source.action_completed(false);
        println!("{}", json!({"event":"decision_stale_before_dispatch"}));
        return Ok(false);
    }
    let Decision::Action { action_id, .. } = decision else {
        return Err(String::from(
            "combat demo requires a model-selected legal action",
        ));
    };
    let action = actions
        .actions()
        .iter()
        .find(|a| a.action_id() == action_id)
        .ok_or_else(|| String::from("model selected an action outside the host catalog"))?;
    let identity = ActionIdentity::new(
        format!("demo-op-{}", steps + 1),
        before.state_id(),
        before.generation(),
        &action_id,
    )
    .map_err(|error| error.to_string())?;
    let action_payload = port
        .current_payload(action)
        .map_err(|error| error.to_string())?;
    // Keep process output bounded and free of provider reasoning, action IDs, and
    // game observations. The corresponding sanitized telemetry spans carry
    // domain-separated digests and the model execution identity.
    println!(
        "{}",
        json!({"event":replay.event(),
            "model_execution_id":source.model_execution_id().unwrap_or(input.execution_id).get(),
            "reused_model_execution":source.model_execution_id().is_some_and(|id| id != input.execution_id),
            "observation_digest":replay::Replay::observation_digest(before),
            "action_digest":replay::Replay::action_digest(&action_payload)})
    );
    let result = port
        .dispatch_action(&identity, action)
        .map_err(|error| error.to_string())
        .and_then(|receipt| settle(port, before, receipt));
    source.action_completed(result.is_ok());
    result?;
    Ok(true)
}

fn settle(
    port: &mut RuntimeV3Port,
    before: &EpisodeObservation,
    mut receipt: TransitionReceipt,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(45);
    while matches!(
        receipt.status(),
        DispatchStatus::Accepted | DispatchStatus::Unknown
    ) && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(200));
        receipt = port
            .reconcile(receipt.operation_id())
            .map_err(|error| error.to_string())?;
    }
    let verified = verify_settlement(before, &receipt).map_err(|error| error.to_string())?;
    println!(
        "{}",
        json!({"event":"action_settled",
        "from_generation":verified.before_generation(),
        "to_generation":verified.after_generation()})
    );
    Ok(())
}
