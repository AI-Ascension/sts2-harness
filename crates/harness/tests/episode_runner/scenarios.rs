// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn extended_campaign_budget_remains_bounded_and_cleans_up_after_long_episode() {
    let config = |steps| {
        EpisodeRunnerConfig::new(
            steps,
            StabilityBarrier::new(2, 1).expect("barrier"),
            RecoveryController::new(1).expect("recovery"),
            "complete the run",
            vec![],
        )
    };
    assert!(config(0).is_err());
    assert!(config(4097).is_err());
    let mut states: Vec<_> = (0..1100)
        .map(|generation| state(EpisodeStage::Combat, generation))
        .collect();
    states.push(state(EpisodeStage::Defeat, 1100));
    let mut runtime = FakeRuntime::new(states);
    let mut model = FakeModel::default();
    let report = EpisodeRunner::new(config(4096).expect("bounded campaign budget"))
        .run(&mut runtime, &mut model)
        .expect("long synthetic episode");
    assert_eq!(report.transitions(), 1100);
    assert_eq!(runtime.dispatches, 1100);
    assert!(runtime.released && runtime.mcp_closed && runtime.gateway_closed);
}

#[test]
fn runner_routes_every_playable_surface_and_verifies_terminal_transition() {
    let mut runtime = FakeRuntime::new(complete_states());
    let mut model = FakeModel::default();
    let runner = runner();
    assert!(!runner.config().map_context_enabled());
    let report = runner
        .run(&mut runtime, &mut model)
        .expect("fake run should complete");
    assert_eq!(report.terminal_stage(), EpisodeStage::Victory);
    assert_eq!(report.transitions(), 8);
    assert_eq!(report.steps(), 8);
    assert_eq!(report.recoveries(), 0);
    assert_eq!(model.calls, 8);
    assert_eq!(model.completions, vec![true; 8]);
    assert_eq!(runtime.dispatches, 8);
    assert!(runtime.launched);
    assert!(runtime.released);
    assert!(runtime.mcp_closed);
    assert!(runtime.gateway_closed);
}

#[test]
fn enabled_map_context_fails_closed_when_runtime_has_no_map_capability() {
    let mut runtime = FakeRuntime::new(vec![
        state(EpisodeStage::Map, 0),
        state(EpisodeStage::Victory, 1),
    ]);
    let mut model = FakeModel::default();
    let config = EpisodeRunnerConfig::new(
        16,
        StabilityBarrier::new(2, 1).expect("barrier"),
        RecoveryController::new(1).expect("recovery"),
        "complete the run",
        vec![String::from("use only current host legal actions")],
    )
    .expect("runner configuration")
    .with_map_context_enabled(true);
    assert!(config.map_context_enabled());
    let error = EpisodeRunner::new(config)
        .run(&mut runtime, &mut model)
        .expect_err("enabled map context must not fall back to the ordinary schema");
    assert!(matches!(
        error,
        EpisodeRunnerError::LegalActions(error) if error.code() == "map_snapshot_unavailable"
    ));
    assert_eq!(model.calls, 0);
    assert!(runtime.released && runtime.mcp_closed && runtime.gateway_closed);
}

#[test]
fn provider_failure_is_fail_closed_and_never_dispatches() {
    let mut runtime = FakeRuntime::new(complete_states());
    let mut model = FakeModel {
        calls: 0,
        unavailable: true,
        ..FakeModel::default()
    };
    let error = runner()
        .run(&mut runtime, &mut model)
        .expect_err("provider failure must stop the run");
    assert!(matches!(
        error,
        EpisodeRunnerError::Policy(PolicyError::ProviderUnavailable)
    ));
    assert_eq!(runtime.dispatches, 0);
    assert!(model.completions.is_empty());
    assert!(runtime.released);
    assert!(runtime.mcp_closed);
    assert!(runtime.gateway_closed);
}

#[test]
fn uncertain_dispatch_is_reconciled_without_a_strategic_retry() {
    let mut runtime = FakeRuntime::new(complete_states());
    runtime.fail_first_dispatch = true;
    let mut model = FakeModel::default();
    let report = runner()
        .run(&mut runtime, &mut model)
        .expect("reconciliation should settle the admitted operation");
    assert_eq!(report.terminal_stage(), EpisodeStage::Victory);
    assert_eq!(runtime.dispatches, 8);
    assert_eq!(runtime.reconciles, 1);
    assert_eq!(runtime.reconciled_operation_ids.len(), 1);
    assert_eq!(
        runtime.reconciled_operation_ids[0],
        runtime.dispatched_operation_ids[0]
    );
    assert!(is_uuid_v4(&runtime.dispatched_operation_ids[0]));
    assert_eq!(runtime.dispatched_state_ids[0], "setup-0");
    assert_eq!(report.recoveries(), 1);
    assert_eq!(model.completions, vec![true; 8]);
}

#[test]
fn fresh_runner_restart_allocates_a_distinct_uuidv4_operation() {
    let mut first_runtime = FakeRuntime::new(complete_states());
    let mut first_model = FakeModel::default();
    runner()
        .run(&mut first_runtime, &mut first_model)
        .expect("first fake run should complete");

    let mut second_runtime = FakeRuntime::new(complete_states());
    let mut second_model = FakeModel::default();
    runner()
        .run(&mut second_runtime, &mut second_model)
        .expect("restarted fake run should complete");

    assert!(is_uuid_v4(&first_runtime.dispatched_operation_ids[0]));
    assert!(is_uuid_v4(&second_runtime.dispatched_operation_ids[0]));
    assert_ne!(
        first_runtime.dispatched_operation_ids[0],
        second_runtime.dispatched_operation_ids[0]
    );
    assert_eq!(first_runtime.dispatched_state_ids[0], "setup-0");
    assert_eq!(second_runtime.dispatched_state_ids[0], "setup-0");
}

#[test]
fn uncertain_dispatch_rejects_reconciliation_for_another_action() {
    for (action_id, kind) in [
        ("other-action", ActionKind::StartRun),
        ("setup-action", ActionKind::EndTurn),
    ] {
        let mut runtime = FakeRuntime::new(complete_states());
        runtime.fail_first_dispatch = true;
        runtime.receipt_action =
            Some(EpisodeLegalAction::new(action_id, kind).expect("valid fake action"));
        assert_conflicting_action_stops(&mut runtime);
    }
}

#[test]
fn accepted_dispatch_rejects_same_id_with_another_action_kind() {
    let mut runtime = FakeRuntime::new(complete_states());
    runtime.receipt_action = Some(
        EpisodeLegalAction::new("setup-action", ActionKind::EndTurn).expect("valid fake action"),
    );
    assert_conflicting_action_stops(&mut runtime);
}

fn assert_conflicting_action_stops(runtime: &mut FakeRuntime) {
    let mut model = FakeModel::default();
    let error = runner()
        .run(runtime, &mut model)
        .expect_err("a different action must not settle the admitted operation");
    assert!(matches!(error, EpisodeRunnerError::ConflictingOperation));
    assert_eq!(runtime.dispatches, 1);
    assert_eq!(runtime.reconciles, 1);
    assert!(is_uuid_v4(&runtime.dispatched_operation_ids[0]));
    assert_eq!(
        runtime.reconciled_operation_ids,
        runtime.dispatched_operation_ids
    );
    assert_eq!(model.calls, 1);
    assert_eq!(model.completions, vec![false]);
    assert!(runtime.released);
    assert!(runtime.mcp_closed);
    assert!(runtime.gateway_closed);
}

pub(super) fn blocked_state(generation: u64) -> State {
    let mut blocked = state(EpisodeStage::Setup, generation);
    blocked.observation = EpisodeObservation::new(
        blocked.observation.state_id(),
        generation,
        EpisodeStage::Setup,
        false,
        true,
        false,
        blocked.observation.fair_play().as_value().clone(),
    )
    .expect("valid blocked state");
    blocked
}

#[test]
fn blocked_input_waits_for_host_readiness_without_calling_policy() {
    let mut runtime = FakeRuntime::new(vec![
        blocked_state(0),
        state(EpisodeStage::Setup, 1),
        state(EpisodeStage::Victory, 2),
    ]);
    runtime.advance_idle = true;
    let mut model = FakeModel::default();
    let report = runner()
        .run(&mut runtime, &mut model)
        .expect("ready successor completes");
    assert_eq!(report.terminal_stage(), EpisodeStage::Victory);
    assert_eq!(runtime.dispatches, 1);
    assert_eq!(model.calls, 1);
}

#[test]
fn blocked_input_timeout_fails_closed_and_cleans_up() {
    let mut runtime = FakeRuntime::new(vec![blocked_state(0)]);
    let mut model = FakeModel::default();
    let error = runner()
        .run(&mut runtime, &mut model)
        .expect_err("input never becomes ready");
    assert!(matches!(
        error,
        EpisodeRunnerError::Barrier(BarrierError::Timeout)
    ));
    assert_eq!(model.calls, 0);
    assert_eq!(runtime.dispatches, 0);
    assert!(runtime.released && runtime.mcp_closed && runtime.gateway_closed);
}

#[test]
fn unfinished_reconciliation_waits_for_the_same_operation_without_redispatch() {
    let mut runtime = FakeRuntime::new(complete_states());
    runtime.unknown_first_receipt = true;
    runtime.unknown_first_reconcile = true;
    let mut model = FakeModel::default();
    let report = runner()
        .run(&mut runtime, &mut model)
        .expect("later host completion");
    assert_eq!(report.transitions(), 8);
    assert_eq!(runtime.dispatches, 8);
    assert_eq!(model.calls, 8);
    assert_eq!(runtime.reconciles, 1);
}

#[test]
fn unresolved_operation_timeout_never_calls_policy_again() {
    let mut runtime = FakeRuntime::new(complete_states());
    runtime.unknown_first_receipt = true;
    runtime.unknown_first_reconcile = true;
    runtime.wait_times_out = true;
    let mut model = FakeModel::default();
    let error = runner()
        .run(&mut runtime, &mut model)
        .expect_err("completion remains unproven");
    assert!(matches!(error, EpisodeRunnerError::UncertainMutation));
    assert_eq!(runtime.dispatches, 1);
    assert_eq!(model.calls, 1);
    assert!(runtime.pending.is_some());
    assert_eq!(model.completions, vec![false]);
    assert!(runtime.released && runtime.mcp_closed && runtime.gateway_closed);
}

fn is_uuid_v4(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| {
        id.get_version_num() == 4
            && id.get_variant() == uuid::Variant::RFC4122
            && id.to_string() == value
    })
}
