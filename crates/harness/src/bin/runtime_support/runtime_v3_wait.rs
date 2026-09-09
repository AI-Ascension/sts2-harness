// SPDX-License-Identifier: MIT

use super::{RuntimeV3Port, parse, recording};
use serde_json::json;
use std::time::{Duration, Instant};
use sts2_harness::{
    BarrierError, BarrierPort, DispatchStatus, EpisodeRuntimePort, OperationState,
    TransitionReceipt, WaitOutcome, WaitSample,
};

impl BarrierPort for RuntimeV3Port {
    fn wait_for_transition(
        &mut self,
        operation_id: &str,
        wait_for_millis: u32,
    ) -> Result<WaitSample, BarrierError> {
        if wait_for_millis == 0 || wait_for_millis > 120_000 {
            return Err(BarrierError::InvalidConfiguration);
        }
        // A durable UNKNOWN/intent-recorded operation has already crossed the mutation
        // boundary without a usable receipt.  The runner must stay in the recovery state until
        // the historical sideband closes it; allowing this ordinary wait to run would turn an
        // unresolved recovery into an uncorrelated gameplay read.
        if self.operations.contains_key(operation_id)
            && let Some(durable) = &self.durable
        {
            let state = durable
                .operation_state(operation_id)
                .map_err(|_| BarrierError::PortFailure)?;
            if matches!(
                state,
                OperationState::IntentRecorded
                    | OperationState::MayHaveBeenDispatched
                    | OperationState::Unknown
            ) {
                return Ok(WaitSample::new(WaitOutcome::RecoveryRequired, None));
            }
        }
        let deadline = Instant::now() + Duration::from_millis(u64::from(wait_for_millis));
        let before_generation = self.generation;
        let mut last_idle_sample = None;
        loop {
            let sample = if self.operations.contains_key(operation_id) {
                if self.is_expert_profile()
                    && self.operations.get(operation_id).is_some_and(|record| {
                        record.action.kind() == sts2_harness::ActionKind::UsePotion
                    })
                {
                    self.poll_expert_operation(operation_id)?
                } else {
                    self.poll_operation(operation_id, wait_for_millis, None, true)?
                }
            } else if operation_id.starts_with("episode-idle-")
                || operation_id.starts_with("episode-wait-")
            {
                // Idle stability observes host state; it cannot manufacture an action witness.
                let observation = self.observe().map_err(|error| {
                    if std::env::var("STS2_LIVE_EPISODE").as_deref() == Ok("true") {
                        // Codes are harness-owned constants. Do not log arbitrary port messages.
                        eprintln!("idle transition observation failed: code={}", error.code());
                    }
                    BarrierError::PortFailure
                })?;
                let idle_sample = (
                    observation.generation(),
                    observation.assert_actionable().is_ok(),
                );
                if last_idle_sample != Some(idle_sample)
                    && std::env::var("STS2_LIVE_EPISODE").as_deref() == Ok("true")
                {
                    eprintln!(
                        "idle transition: operation={operation_id} before={before_generation} observed={} actionable={}",
                        observation.generation(),
                        observation.assert_actionable().is_ok()
                    );
                }
                last_idle_sample = Some(idle_sample);
                if observation.generation() > before_generation {
                    WaitSample::new(WaitOutcome::Successor, Some(observation))
                } else {
                    WaitSample::new(WaitOutcome::Timeout, None)
                }
            } else {
                return Err(BarrierError::InvalidOperation);
            };
            if sample.outcome() != WaitOutcome::Timeout || Instant::now() >= deadline {
                return Ok(sample);
            }
            std::thread::sleep(
                Duration::from_millis(100).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
}

impl RuntimeV3Port {
    /// Fetches the ordinary host transition witness after the recovery sideband has already
    /// returned a terminal authoritative record.  This is deliberately not part of
    /// `BarrierPort`: callers cannot use it to bypass the unresolved-operation recovery gate.
    pub(super) fn recovery_wait_for_settled_operation(
        &mut self,
        operation_id: &str,
        minimum_generation: u64,
        wait_for_millis: u32,
    ) -> Result<TransitionReceipt, String> {
        if wait_for_millis == 0 || wait_for_millis > 120_000 {
            return Err(String::from("invalid retained-witness wait budget"));
        }
        let record = self
            .operations
            .get(operation_id)
            .cloned()
            .ok_or_else(|| String::from("recovery operation is not retained"))?;
        let sample = self
            .poll_operation(
                operation_id,
                wait_for_millis,
                Some(minimum_generation),
                false,
            )
            .map_err(|error| format!("post-recovery gameplay witness failed: {error:?}"))?;
        if !matches!(
            sample.outcome(),
            WaitOutcome::Successor | WaitOutcome::SameStateMutation
        ) {
            return Err(String::from(
                "post-recovery gameplay witness did not settle",
            ));
        }
        let after = sample
            .observation()
            .cloned()
            .ok_or_else(|| String::from("post-recovery gameplay witness omitted observation"))?;
        if after.generation() < minimum_generation || after.generation() <= record.generation {
            return Err(String::from(
                "post-recovery gameplay observation is older than the historical witness",
            ));
        }
        let effect_kind = sample
            .effect_kind()
            .map(str::to_owned)
            .ok_or_else(|| String::from("post-recovery gameplay witness omitted effect"))?;
        Ok(TransitionReceipt::new(
            operation_id,
            record.action,
            DispatchStatus::Settled,
            Some(after),
            Some(effect_kind),
            None,
        ))
    }

    fn poll_expert_operation(&mut self, operation_id: &str) -> Result<WaitSample, BarrierError> {
        let record = self
            .operations
            .get(operation_id)
            .cloned()
            .ok_or(BarrierError::InvalidOperation)?;
        let sample = self
            .wait_expert_operation(operation_id)
            .map_err(|_| BarrierError::PortFailure)?;
        self.update_durable_wait(operation_id, &sample, None)?;
        recording::wait(
            operation_id,
            record.action.action_id(),
            record.generation,
            &sample,
            &self.telemetry,
        );
        Ok(sample)
    }

    fn poll_operation(
        &mut self,
        operation_id: &str,
        wait_for_millis: u32,
        minimum_generation: Option<u64>,
        update_durable: bool,
    ) -> Result<WaitSample, BarrierError> {
        let generation = self
            .operations
            .get(operation_id)
            .ok_or(BarrierError::InvalidOperation)?
            .generation;
        let action_id = self
            .operations
            .get(operation_id)
            .map(|record| record.action.action_id().to_owned())
            .ok_or(BarrierError::InvalidOperation)?;
        let (value, response_text) = self.call_tool_with_text("sts2.wait_for_transition", json!({
            "instance_id":self.config.instance_id, "mcp_session_id":self.config.mcp_session_id,
            "lease_id":self.config.lease_id, "lease_epoch":self.config.lease_epoch,
            "generation":self.generation, "operation_id":operation_id,
            "wait_for_millis":wait_for_millis
        })).map_err(|_| BarrierError::PortFailure)?;
        let sample = parse::wait_sample(
            &value,
            &response_text,
            &self.config,
            operation_id,
            generation,
        )
        .map_err(|_| BarrierError::PortFailure)?;
        if minimum_generation.is_some_and(|minimum| {
            sample
                .observation()
                .is_some_and(|observation| observation.generation() < minimum)
        }) {
            // Do not checkpoint or settle a durable operation from an observation that predates
            // the authoritative sideband witness.  The durable state must remain unresolved so
            // a caller can retry the recovery read or quarantine it.
            return Err(BarrierError::StaleObservation);
        }
        self.install_response(&value, &response_text, "wait_response")
            .map_err(|_| BarrierError::PortFailure)?;
        if update_durable {
            self.update_durable_wait(operation_id, &sample, Some(&value))?;
        }
        let sample = if self.is_expert_profile() {
            self.compose_wait_sample(sample)
                .map_err(|_| BarrierError::PortFailure)?
        } else {
            sample
        };
        recording::wait(
            operation_id,
            &action_id,
            generation,
            &sample,
            &self.telemetry,
        );
        Ok(sample)
    }

    fn update_durable_wait(
        &self,
        operation_id: &str,
        sample: &WaitSample,
        response: Option<&serde_json::Value>,
    ) -> Result<(), BarrierError> {
        let Some(durable) = &self.durable else {
            return Ok(());
        };
        let state = durable
            .operation_state(operation_id)
            .map_err(|_| BarrierError::PortFailure)?;
        match sample.outcome() {
            WaitOutcome::Successor | WaitOutcome::SameStateMutation => {
                if matches!(
                    state,
                    OperationState::Unknown | OperationState::IntentRecorded
                ) {
                    if let Some(response) = response {
                        durable
                            .reconcile_response(operation_id, OperationState::Settled, response)
                            .map_err(|_| BarrierError::PortFailure)?;
                    } else {
                        let digest = durable
                            .operation_payload_digest(operation_id)
                            .map_err(|_| BarrierError::InvalidOperation)?;
                        durable
                            .operation_result(operation_id, &digest, OperationState::Settled, None)
                            .map_err(|_| BarrierError::PortFailure)?;
                    }
                } else if state != OperationState::Reconciled {
                    let digest = durable
                        .operation_payload_digest(operation_id)
                        .map_err(|_| BarrierError::InvalidOperation)?;
                    durable
                        .operation_result(operation_id, &digest, OperationState::Settled, response)
                        .map_err(|_| BarrierError::PortFailure)?;
                }
            }
            WaitOutcome::RecoveryRequired => {
                if state == OperationState::Accepted {
                    let digest = durable
                        .operation_payload_digest(operation_id)
                        .map_err(|_| BarrierError::InvalidOperation)?;
                    durable
                        .operation_result(operation_id, &digest, OperationState::Unknown, None)
                        .map_err(|_| BarrierError::PortFailure)?;
                }
            }
            WaitOutcome::Timeout => {}
        }
        Ok(())
    }
}
