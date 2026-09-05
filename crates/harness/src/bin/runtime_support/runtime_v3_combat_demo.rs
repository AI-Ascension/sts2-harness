// SPDX-License-Identifier: MIT

use super::RuntimeV3Port;
#[path = "runtime_v3_combat_replay.rs"]
mod replay;
use serde_json::json;
use std::time::{Duration, Instant};
use sts2_harness::{
    ActionIdentity, Decision, DecisionInput, DecisionSource, DispatchStatus, EpisodeObservation,
    EpisodeRunnerConfig, EpisodeRuntimePort, EpisodeShutdown, EpisodeStage, ModelExecutionId,
    RecoveryPort, TransitionReceipt, verify_settlement,
};

pub(super) fn run<S: DecisionSource>(
    port: &mut RuntimeV3Port,
    source: &mut S,
    config: &EpisodeRunnerConfig,
) -> Result<(), String> {
    port.launch().map_err(|error| error.to_string())?;
    let result = run_inner(port, source, config);
    let cleanup = EpisodeShutdown
        .close(port)
        .map_err(|error| error.to_string());
    result.and(cleanup)
}

fn run_inner<S: DecisionSource>(
    port: &mut RuntimeV3Port,
    source: &mut S,
    config: &EpisodeRunnerConfig,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(900);
    let mut steps = 0_u32;
    let mut saw_combat = false;
    let replay = replay::Replay::load()?;
    while Instant::now() < deadline && steps < config.max_steps() {
        let before = port.observe().map_err(|error| error.to_string())?;
        saw_combat |= before.stage() == EpisodeStage::Combat;
        if saw_combat && matches!(before.stage(), EpisodeStage::Reward | EpisodeStage::Defeat) {
            replay.finish(steps, &before)?;
            println!(
                "{}",
                json!({"event":"combat_demo_complete", "steps":steps,
                "stage":format!("{:?}",before.stage()), "observation":before.fair_play().as_value()})
            );
            return Ok(());
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
        println!("{}", json!({"event":"decision_stale_before_dispatch"}));
        return Ok(false);
    }
    let Decision::Action {
        action_id,
        rationale,
        ..
    } = decision
    else {
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
    println!(
        "{}",
        json!({"event":replay.event(), "operation_id":identity.operation_id,
            "action_id":action_id, "rationale":rationale, "observation":before.fair_play().as_value()})
    );
    let receipt = port
        .dispatch_action(&identity, action)
        .map_err(|error| error.to_string())?;
    settle(port, before, receipt)?;
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
        json!({"event":"action_settled", "operation_id":verified.operation_id(),
        "action_id":verified.action_id(), "from_generation":verified.before_generation(),
        "to_generation":verified.after_generation(), "effect":verified.effect_kind(),
        "observation":receipt.after().map(|value| value.fair_play().as_value())})
    );
    Ok(())
}
