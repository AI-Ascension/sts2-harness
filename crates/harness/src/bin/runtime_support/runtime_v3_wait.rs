// SPDX-License-Identifier: MIT

use super::{RuntimeV3Port, parse, recording};
use serde_json::json;
use std::time::{Duration, Instant};
use sts2_harness::{BarrierError, BarrierPort, EpisodeRuntimePort, WaitOutcome, WaitSample};

impl BarrierPort for RuntimeV3Port {
    fn wait_for_transition(
        &mut self,
        operation_id: &str,
        wait_for_millis: u32,
    ) -> Result<WaitSample, BarrierError> {
        if wait_for_millis == 0 || wait_for_millis > 120_000 {
            return Err(BarrierError::InvalidConfiguration);
        }
        let deadline = Instant::now() + Duration::from_millis(u64::from(wait_for_millis));
        let before_generation = self.generation;
        let mut last_idle_sample = None;
        loop {
            let sample = if self.operations.contains_key(operation_id) {
                if self.is_expert_profile()
                    && self.operations.get(operation_id).is_some_and(|record| {
                        self.uses_expert_transport(&record.action, &record.payload)
                    })
                {
                    self.poll_expert_operation(operation_id)?
                } else {
                    self.poll_operation(operation_id, wait_for_millis)?
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
    fn poll_expert_operation(&mut self, operation_id: &str) -> Result<WaitSample, BarrierError> {
        let record = self
            .operations
            .get(operation_id)
            .cloned()
            .ok_or(BarrierError::InvalidOperation)?;
        let sample = self
            .wait_expert_operation(operation_id)
            .map_err(|_| BarrierError::PortFailure)?;
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
        let value = self.call_tool("sts2.wait_for_transition", json!({
            "instance_id":self.config.instance_id, "mcp_session_id":self.config.mcp_session_id,
            "lease_id":self.config.lease_id, "lease_epoch":self.config.lease_epoch,
            "generation":self.generation, "operation_id":operation_id,
            "wait_for_millis":wait_for_millis
        })).map_err(|_| BarrierError::PortFailure)?;
        let sample = parse::wait_sample(&value, &self.config, operation_id, generation)
            .map_err(|_| BarrierError::PortFailure)?;
        self.install_response(&value, "wait_response")
            .map_err(|_| BarrierError::PortFailure)?;
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
}
