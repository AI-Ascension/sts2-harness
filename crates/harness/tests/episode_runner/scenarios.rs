// SPDX-License-Identifier: MIT

use super::*;

struct FailureRuntime {
    inner: FakeRuntime,
    dispatch_error: Option<PortError>,
    reconcile_errors: Vec<RecoveryError>,
    cleanup_errors: [Option<ShutdownError>; 3],
}

impl FailureRuntime {
    fn new(states: Vec<State>) -> Self {
        Self {
            inner: FakeRuntime::new(states),
            dispatch_error: None,
            reconcile_errors: Vec::new(),
            cleanup_errors: [None, None, None],
        }
    }
}

impl EpisodeRuntimePort for FailureRuntime {
    fn launch(&mut self) -> Result<(), PortError> {
        self.inner.launch()
    }

    fn observe(&mut self) -> Result<EpisodeObservation, PortError> {
        self.inner.observe()
    }

    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, PortError> {
        self.inner.legal_actions(state_id, generation)
    }

    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, PortError> {
        let receipt = self.inner.dispatch_action(identity, action)?;
        self.dispatch_error.take().map_or(Ok(receipt), Err)
    }
}

impl BarrierPort for FailureRuntime {
    fn wait_for_transition(
        &mut self,
        operation_id: &str,
        wait_for_millis: u32,
    ) -> Result<WaitSample, BarrierError> {
        self.inner
            .wait_for_transition(operation_id, wait_for_millis)
    }
}

impl RecoveryPort for FailureRuntime {
    fn reobserve(&mut self) -> Result<EpisodeObservation, RecoveryError> {
        self.inner.reobserve()
    }

    fn reconcile(&mut self, operation_id: &str) -> Result<TransitionReceipt, RecoveryError> {
        self.reconcile_errors
            .pop()
            .map_or_else(|| self.inner.reconcile(operation_id), Err)
    }

    fn release_lease(&mut self) -> Result<(), RecoveryError> {
        RecoveryPort::release_lease(&mut self.inner)
    }

    fn stop_episode(&mut self) -> Result<(), RecoveryError> {
        self.inner.stop_episode()
    }
}

impl ShutdownPort for FailureRuntime {
    fn release_lease(&mut self) -> Result<(), ShutdownError> {
        let result = ShutdownPort::release_lease(&mut self.inner);
        self.cleanup_errors[0].take().map_or(result, Err)
    }

    fn close_mcp(&mut self) -> Result<(), ShutdownError> {
        let result = self.inner.close_mcp();
        self.cleanup_errors[1].take().map_or(result, Err)
    }

    fn close_gateway(&mut self) -> Result<(), ShutdownError> {
        let result = self.inner.close_gateway();
        self.cleanup_errors[2].take().map_or(result, Err)
    }
}

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
    assert_eq!(report.recoveries(), 1);
    assert_eq!(model.completions, vec![true; 8]);
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

#[test]
fn legacy_runtime_is_reusable_through_focused_protected_ports() {
    fn assert_protected<P: sts2_harness::ProtectedEpisodePort>() {}
    fn assert_action<P: sts2_harness::EpisodeActionPort>() {}

    assert_protected::<FakeRuntime>();
    assert_action::<FakeRuntime>();
    let mut runtime = FakeRuntime::new(vec![
        state(EpisodeStage::Combat, 0),
        state(EpisodeStage::Victory, 1),
    ]);
    let observation =
        sts2_harness::EpisodeObservationPort::observe(&mut runtime).expect("observation port");
    let actions = sts2_harness::EpisodeObservationPort::legal_actions(
        &mut runtime,
        observation.state_id(),
        observation.generation(),
    )
    .expect("catalog port");
    sts2_harness::EpisodeLifecyclePort::launch(&mut runtime).expect("lifecycle port");
    let action = actions.actions().first().expect("catalog action");
    let identity = ActionIdentity::new(
        "focused-operation",
        observation.state_id(),
        observation.generation(),
        action.action_id(),
    )
    .expect("operation identity");
    sts2_harness::EpisodeActionPort::dispatch_action(&mut runtime, &identity, action)
        .expect("action port");
    assert_eq!(actions.state_id(), observation.state_id());
}

#[test]
fn successful_run_keeps_the_historical_first_cleanup_error() {
    let mut runtime = FailureRuntime::new(complete_states());
    runtime.cleanup_errors = [
        Some(ShutdownError::ReleaseFailed),
        Some(ShutdownError::McpCloseFailed),
        None,
    ];
    let mut model = FakeModel::default();

    let error = runner()
        .run(&mut runtime, &mut model)
        .expect_err("cleanup failure must be reported");
    assert!(matches!(
        error,
        EpisodeRunnerError::Shutdown(ShutdownError::ReleaseFailed)
    ));
    assert!(runtime.inner.released && runtime.inner.mcp_closed && runtime.inner.gateway_closed);
}

#[test]
fn dispatch_and_cleanup_failures_preserve_both_causes_and_operation_identity() {
    let mut runtime = FailureRuntime::new(complete_states());
    runtime.dispatch_error = Some(PortError::new(
        "dispatch_failed",
        "synthetic dispatch",
        false,
    ));
    runtime.reconcile_errors = vec![RecoveryError::PortFailure];
    runtime.cleanup_errors = [
        Some(ShutdownError::ReleaseFailed),
        Some(ShutdownError::McpCloseFailed),
        None,
    ];
    let mut model = FakeModel::default();

    let error = runner()
        .run(&mut runtime, &mut model)
        .expect_err("dispatch and cleanup failures must be reported");
    match error {
        EpisodeRunnerError::Cleanup(failure) => {
            assert_eq!(failure.pending_operation_id(), Some("episode-action-0-1"));
            assert_eq!(
                failure.cleanup().failures(),
                &[ShutdownError::ReleaseFailed, ShutdownError::McpCloseFailed]
            );
            match failure.primary() {
                EpisodeRunnerError::DispatchRecovery {
                    operation_id,
                    dispatch,
                    recovery,
                } => {
                    assert_eq!(operation_id, "episode-action-0-1");
                    assert_eq!(dispatch.code(), "dispatch_failed");
                    assert!(matches!(
                        recovery.as_ref(),
                        EpisodeRunnerError::Recovery(RecoveryError::Exhausted)
                    ));
                }
                other => assert!(matches!(other, EpisodeRunnerError::Dispatch(_))),
            }
        }
        other => assert!(matches!(other, EpisodeRunnerError::Cleanup(_))),
    }
    assert!(runtime.inner.released && runtime.inner.mcp_closed && runtime.inner.gateway_closed);
}
