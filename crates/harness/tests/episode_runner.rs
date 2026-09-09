// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use serde_json::{Value, json};
use sts2_harness::{
    ActionIdentity, ActionKind, BarrierError, BarrierPort, Decision, DecisionInput, DecisionSource,
    DispatchStatus, EpisodeLegalAction, EpisodeLegalActionSet, EpisodeObservation, EpisodeRunner,
    EpisodeRunnerConfig, EpisodeRunnerError, EpisodeRuntimePort, EpisodeStage, PolicyError,
    PortError, RecoveryController, RecoveryError, RecoveryPort, ShutdownError, ShutdownPort,
    StabilityBarrier, TransitionReceipt, WaitOutcome, WaitSample,
};

#[derive(Clone)]
struct State {
    observation: EpisodeObservation,
    actions: EpisodeLegalActionSet,
}

struct PendingTransition {
    operation_id: String,
    action: EpisodeLegalAction,
    after: EpisodeObservation,
}

struct FakeRuntime {
    states: Vec<State>,
    index: usize,
    pending: Option<PendingTransition>,
    fail_first_dispatch: bool,
    unknown_first_receipt: bool,
    unknown_first_reconcile: bool,
    advance_idle: bool,
    wait_times_out: bool,
    receipt_action: Option<EpisodeLegalAction>,
    dispatches: usize,
    reconciles: usize,
    launched: bool,
    released: bool,
    fail_release: bool,
    mcp_closed: bool,
    gateway_closed: bool,
    dispatched_operation_ids: Vec<String>,
    reconciled_operation_ids: Vec<String>,
    dispatched_state_ids: Vec<String>,
    catalog_errors: Vec<PortError>,
    catalog_calls: usize,
    catalog_requests: Vec<(String, u64)>,
    advance_catalog: bool,
    idle_waits: usize,
    reobserve_calls: usize,
    reobserve_errors: Vec<RecoveryError>,
    observe_after_reobserve: Option<Result<EpisodeObservation, PortError>>,
}

impl FakeRuntime {
    fn new(states: Vec<State>) -> Self {
        Self {
            states,
            index: 0,
            pending: None,
            fail_first_dispatch: false,
            unknown_first_receipt: false,
            unknown_first_reconcile: false,
            advance_idle: false,
            wait_times_out: false,
            receipt_action: None,
            dispatches: 0,
            reconciles: 0,
            launched: false,
            released: false,
            fail_release: false,
            mcp_closed: false,
            gateway_closed: false,
            dispatched_operation_ids: Vec::new(),
            reconciled_operation_ids: Vec::new(),
            dispatched_state_ids: Vec::new(),
            catalog_errors: Vec::new(),
            catalog_calls: 0,
            catalog_requests: Vec::new(),
            advance_catalog: false,
            idle_waits: 0,
            reobserve_calls: 0,
            reobserve_errors: Vec::new(),
            observe_after_reobserve: None,
        }
    }

    fn current(&self) -> &State {
        &self.states[self.index]
    }

    fn next_observation(&self) -> Option<EpisodeObservation> {
        self.states
            .get(self.index + 1)
            .map(|state| state.observation.clone())
    }
}

impl EpisodeRuntimePort for FakeRuntime {
    fn launch(&mut self) -> Result<(), PortError> {
        self.launched = true;
        Ok(())
    }

    fn observe(&mut self) -> Result<EpisodeObservation, PortError> {
        if self.reobserve_calls > 0
            && let Some(result) = self.observe_after_reobserve.take()
        {
            return result;
        }
        Ok(self.current().observation.clone())
    }

    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, PortError> {
        self.catalog_calls += 1;
        self.catalog_requests
            .push((state_id.to_owned(), generation));
        if !self.catalog_errors.is_empty() {
            if self.advance_catalog {
                self.index += 1;
            }
            return Err(self.catalog_errors.remove(0));
        }
        let state = self.current();
        if state.observation.state_id() != state_id || state.observation.generation() != generation
        {
            return Err(PortError::new(
                "stale_catalog",
                "fake catalog identity does not match observation",
                false,
            ));
        }
        Ok(state.actions.clone())
    }

    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, PortError> {
        if identity.state_id != self.current().observation.state_id()
            || identity.generation != self.current().observation.generation()
        {
            return Err(PortError::new(
                "wrong_state_identity",
                "fake runtime received a non-authoritative state identity",
                false,
            ));
        }
        if self.pending.is_some() {
            return Err(PortError::new(
                "pending_transition",
                "fake runtime already has a pending transition",
                false,
            ));
        }
        let after = self.next_observation().ok_or_else(|| {
            PortError::new("terminal_runtime", "fake runtime has no successor", false)
        })?;
        self.pending = Some(PendingTransition {
            operation_id: identity.operation_id.clone(),
            action: action.clone(),
            after,
        });
        self.dispatched_operation_ids
            .push(identity.operation_id.clone());
        self.dispatched_state_ids.push(identity.state_id.clone());
        self.dispatches += 1;
        if self.fail_first_dispatch && self.dispatches == 1 {
            return Err(PortError::new(
                "transport_uncertain",
                "fake transport failed after admission",
                true,
            ));
        }
        Ok(TransitionReceipt::new(
            identity.operation_id.clone(),
            self.receipt_action
                .clone()
                .unwrap_or_else(|| action.clone()),
            if self.unknown_first_receipt && self.dispatches == 1 {
                DispatchStatus::Unknown
            } else {
                DispatchStatus::Accepted
            },
            None,
            None,
            None,
        ))
    }
}

impl BarrierPort for FakeRuntime {
    fn wait_for_transition(
        &mut self,
        operation_id: &str,
        _wait_for_millis: u32,
    ) -> Result<WaitSample, BarrierError> {
        if self.wait_times_out {
            return Ok(WaitSample::new(WaitOutcome::Timeout, None));
        }
        if self.advance_idle
            && self.pending.is_none()
            && operation_id.starts_with("episode-idle-")
            && let Some(after) = self.next_observation()
        {
            self.idle_waits += 1;
            self.index += 1;
            return Ok(WaitSample::new(WaitOutcome::Successor, Some(after)));
        }
        let Some(pending) = self.pending.take() else {
            return Ok(WaitSample::new(WaitOutcome::Timeout, None));
        };
        if pending.operation_id != operation_id {
            return Err(BarrierError::InvalidOperation);
        }
        self.index += 1;
        Ok(WaitSample::new(WaitOutcome::Successor, Some(pending.after))
            .with_effect_kind("host.semantic.settled"))
    }
}

impl RecoveryPort for FakeRuntime {
    fn reobserve(&mut self) -> Result<EpisodeObservation, RecoveryError> {
        self.reobserve_calls += 1;
        if !self.reobserve_errors.is_empty() {
            return Err(self.reobserve_errors.remove(0));
        }
        Ok(self.current().observation.clone())
    }

    fn reconcile(&mut self, operation_id: &str) -> Result<TransitionReceipt, RecoveryError> {
        self.reconciled_operation_ids.push(operation_id.to_owned());
        if self.unknown_first_reconcile && self.reconciles == 0 {
            self.reconciles += 1;
            let pending = self
                .pending
                .as_ref()
                .ok_or(RecoveryError::InvalidOperation)?;
            if pending.operation_id != operation_id {
                return Err(RecoveryError::InvalidOperation);
            }
            return Ok(TransitionReceipt::new(
                operation_id,
                pending.action.clone(),
                DispatchStatus::Unknown,
                None,
                None,
                Some("still_executing".into()),
            ));
        }
        let Some(pending) = self.pending.take() else {
            return Err(RecoveryError::PortFailure);
        };
        if pending.operation_id != operation_id {
            return Err(RecoveryError::InvalidOperation);
        }
        self.index += 1;
        self.reconciles += 1;
        Ok(TransitionReceipt::new(
            pending.operation_id,
            self.receipt_action.clone().unwrap_or(pending.action),
            DispatchStatus::Settled,
            Some(pending.after),
            Some(String::from("host.semantic.reconciled")),
            None,
        ))
    }

    fn release_lease(&mut self) -> Result<(), RecoveryError> {
        self.released = true;
        Ok(())
    }

    fn stop_episode(&mut self) -> Result<(), RecoveryError> {
        Ok(())
    }
}

impl ShutdownPort for FakeRuntime {
    fn release_lease(&mut self) -> Result<(), ShutdownError> {
        self.released = true;
        if self.fail_release {
            Err(ShutdownError::ReleaseFailed)
        } else {
            Ok(())
        }
    }

    fn close_mcp(&mut self) -> Result<(), ShutdownError> {
        self.mcp_closed = true;
        Ok(())
    }

    fn close_gateway(&mut self) -> Result<(), ShutdownError> {
        self.gateway_closed = true;
        Ok(())
    }
}

#[derive(Default)]
struct FakeModel {
    calls: usize,
    unavailable: bool,
    completions: Vec<bool>,
    input_generations: Vec<u64>,
}

impl DecisionSource for FakeModel {
    fn action_completed(&mut self, settled: bool) {
        self.completions.push(settled);
    }

    fn decide(&mut self, input: &DecisionInput) -> Result<Decision, PolicyError> {
        self.calls += 1;
        self.input_generations.push(input.observation.generation());
        if self.unavailable {
            return Err(PolicyError::ProviderUnavailable);
        }
        let action = input
            .legal_actions
            .actions()
            .first()
            .ok_or(PolicyError::IllegalAction)?;
        Ok(Decision::Action {
            action_id: action.action_id().to_owned(),
            rationale: String::from("bounded fake provider decision"),
            confidence: Some(75),
        })
    }
}

#[path = "episode_runner/fixtures.rs"]
mod fixtures;
use fixtures::{complete_states, runner, state};

#[path = "episode_runner/scenarios.rs"]
mod scenarios;

#[path = "episode_runner/catalog_reobserve.rs"]
mod catalog_reobserve;

#[path = "episode_runner/completion_ordering.rs"]
mod completion_ordering;
