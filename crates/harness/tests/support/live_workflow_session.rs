// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]
#![allow(dead_code)]

use sts2_harness::management::LiveWorkflowSession;
use sts2_harness::{
    ActionIdentity, ActionKind, Decision, DecisionInput, DispatchStatus, EpisodeLegalAction,
    EpisodeLegalActionSet, EpisodeObservation, TransitionReceipt, WaitOutcome, WaitSample,
};

use super::{FakeSession, observation};

impl LiveWorkflowSession for FakeSession {
    fn launch(&mut self) -> Result<(), sts2_harness::management::ManagementError> {
        self.record("launch");
        if self.launch_error {
            return Err(sts2_harness::management::ManagementError::unavailable(
                "fake_launch",
                "fixture launch failed",
            ));
        }
        Ok(())
    }

    fn observe(&mut self) -> Result<EpisodeObservation, sts2_harness::management::ManagementError> {
        self.record("observe");
        Ok(observation("state-0", 0))
    }

    fn observe_projection(
        &mut self,
        projection_ref: &str,
    ) -> Result<EpisodeObservation, sts2_harness::management::ManagementError> {
        assert_eq!(projection_ref, "fair-play.live.v1");
        self.observe()
    }

    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, sts2_harness::management::ManagementError> {
        self.record("legal_actions");
        self.catalog_calls = self.catalog_calls.saturating_add(1);
        let mut actions =
            vec![EpisodeLegalAction::new("end-turn", ActionKind::EndTurn).expect("action")];
        if self.catalog_drift && self.catalog_calls > 1 {
            actions
                .push(EpisodeLegalAction::new("play-card", ActionKind::PlayCard).expect("action"));
        }
        EpisodeLegalActionSet::new(state_id, generation, actions).map_err(|error| {
            sts2_harness::management::ManagementError::invalid("fake_catalog", error.to_string())
        })
    }

    fn decide(
        &mut self,
        _input: &DecisionInput,
    ) -> Result<Decision, sts2_harness::management::ManagementError> {
        self.record("decide");
        if self.decide_unknown {
            return Err(sts2_harness::management::ManagementError::unresolved(
                "fake_decide_transport",
                "decision reply was lost after the provider exchange",
            ));
        }
        if self.decide_error {
            return Err(sts2_harness::management::ManagementError::capability(
                "fake_decide",
                "fixture decision failed before any provider call",
            ));
        }
        Ok(Decision::Action {
            action_id: "end-turn".to_owned(),
            rationale: "fixture decision".to_owned(),
            confidence: Some(100),
        })
    }

    fn decide_for(
        &mut self,
        input: &DecisionInput,
        decision_profile_ref: &str,
        context_ref: &str,
    ) -> Result<Decision, sts2_harness::management::ManagementError> {
        assert_eq!(decision_profile_ref, "decision.live.v1");
        assert_eq!(context_ref, "context.live.v1");
        self.decide(input)
    }

    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, sts2_harness::management::ManagementError> {
        self.record("dispatch");
        self.identity = Some(identity.operation_id.clone());
        self.action = Some(action.clone());
        if self.dispatch_error {
            return Err(sts2_harness::management::ManagementError::unresolved(
                "fake_transport",
                "dispatch response was lost",
            ));
        }
        if self.mismatched_receipt {
            return Ok(TransitionReceipt::new(
                format!("{}-other", identity.operation_id),
                action.clone(),
                DispatchStatus::Settled,
                Some(observation("state-1", 1)),
                Some("host.semantic.mismatched".to_owned()),
                None,
            ));
        }
        let status = if self.unknown {
            DispatchStatus::Unknown
        } else {
            DispatchStatus::Accepted
        };
        Ok(TransitionReceipt::new(
            identity.operation_id.clone(),
            action.clone(),
            status,
            None,
            None,
            None,
        ))
    }

    fn wait_for_transition(
        &mut self,
        _operation_id: &str,
        _wait_for_millis: u32,
    ) -> Result<WaitSample, sts2_harness::management::ManagementError> {
        self.record("wait");
        Ok(
            WaitSample::new(WaitOutcome::Successor, Some(observation("state-1", 1)))
                .with_effect_kind("host.semantic.test"),
        )
    }

    fn reconcile(
        &mut self,
        operation_id: &str,
    ) -> Result<TransitionReceipt, sts2_harness::management::ManagementError> {
        self.record("reconcile");
        let action = self.action.clone().ok_or_else(|| {
            sts2_harness::management::ManagementError::unavailable(
                "fake_reconcile",
                "missing operation",
            )
        })?;
        if self.identity.as_deref() != Some(operation_id) {
            return Err(sts2_harness::management::ManagementError::conflict(
                "fake_reconcile",
                "operation identity mismatch",
            ));
        }
        if self.reconcile_conflict {
            return Ok(TransitionReceipt::new(
                format!("{operation_id}-other"),
                action,
                DispatchStatus::Settled,
                Some(observation("state-1", 1)),
                Some("host.semantic.reconcile-conflict".to_owned()),
                None,
            ));
        }
        if self.reconcile_unknown {
            return Ok(TransitionReceipt::new(
                operation_id,
                action,
                DispatchStatus::Accepted,
                None,
                None,
                None,
            ));
        }
        if let Some(status) = self.reconcile_status {
            return Ok(TransitionReceipt::new(
                operation_id,
                action,
                status,
                (status == DispatchStatus::Settled).then(|| observation("state-1", 1)),
                (status == DispatchStatus::Settled).then(|| "host.semantic.reconciled".to_owned()),
                None,
            ));
        }
        Ok(TransitionReceipt::new(
            operation_id,
            action,
            DispatchStatus::Settled,
            Some(observation("state-1", 1)),
            Some("host.semantic.reconciled".to_owned()),
            None,
        ))
    }

    fn release_lease(&mut self) -> Result<(), sts2_harness::management::ManagementError> {
        self.record("release");
        if self.release_error {
            return Err(sts2_harness::management::ManagementError::unavailable(
                "fake_release",
                "fixture lease release failed",
            ));
        }
        Ok(())
    }

    fn stop_episode(&mut self) -> Result<(), sts2_harness::management::ManagementError> {
        self.record("stop");
        if self.stop_error {
            return Err(sts2_harness::management::ManagementError::unavailable(
                "fake_stop",
                "fixture stop failed",
            ));
        }
        Ok(())
    }

    fn action_completed(&mut self, settled: bool) {
        self.completions
            .lock()
            .expect("completion log")
            .push(settled);
    }
}
