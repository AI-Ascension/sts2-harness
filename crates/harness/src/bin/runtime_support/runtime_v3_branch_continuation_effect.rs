// SPDX-License-Identifier: MIT

struct RuntimeBranchContinuationEffectPort<'a> {
    port: &'a mut RuntimeV3Port,
    runner: &'a EpisodeRunnerConfig,
    continuation: &'a mut dyn DecisionSource,
}

struct DecisionSourceAdapter<'a>(&'a mut dyn DecisionSource);

impl DecisionSource for DecisionSourceAdapter<'_> {
    fn decide(&mut self, input: &DecisionInput) -> Result<Decision, PolicyError> {
        self.0.decide(input)
    }

    fn decide_with_game_information(
        &mut self,
        input: &DecisionInput,
        runtime: &mut dyn sts2_harness::EpisodeRuntimePort,
    ) -> Result<Decision, PolicyError> {
        self.0.decide_with_game_information(input, runtime)
    }

    fn decide_for(
        &mut self,
        input: &DecisionInput,
        decision_profile_ref: &str,
        context_ref: &str,
    ) -> Result<Decision, PolicyError> {
        self.0.decide_for(input, decision_profile_ref, context_ref)
    }

    fn action_completed(&mut self, settled: bool) {
        self.0.action_completed(settled);
    }

    fn model_execution_id(&self) -> Option<ModelExecutionId> {
        self.0.model_execution_id()
    }

    fn close(&mut self) -> Result<(), PolicyError> {
        self.0.close()
    }
}

impl BranchContinuationEffectPort for RuntimeBranchContinuationEffectPort<'_> {
    type Output = episode_replay::ReplayOutcome;

    fn exact_restore(
        &mut self,
        selected: &mut branch_runtime::SelectedBranchContinuation,
    ) -> Result<Self::Output, String> {
        if selected.branch().assurance != sts2_harness::BranchAssurance::ExactRestoreReceipt
            || selected.branch().status != sts2_harness::DurableBranchStatus::Running
        {
            return Err(String::from(
                "exact gameplay continuation requires a persisted verified restore receipt and running branch",
            ));
        }
        let mut continuation = DecisionSourceAdapter(self.continuation);
        sts2_harness::EpisodeRunner::new(self.runner.clone())
            .run(self.port, &mut continuation)
            .map_err(|error| error.to_string())
            .and_then(|report| {
                if report.terminal_stage() == EpisodeStage::Unknown {
                    return Err(String::from(
                        "exact gameplay continuation ended at an unknown stage",
                    ));
                }
                Ok(episode_replay::ReplayOutcome::Terminal {
                    stage: report.terminal_stage(),
                    observation: report.final_observation().clone(),
                })
            })
    }

    fn prefix_replay(
        &mut self,
        selected: &mut branch_runtime::SelectedBranchContinuation,
        prefix: &[u8],
    ) -> Result<Self::Output, String> {
        selected.claim_prefix_replay()?;
        let mut publish_boundary = || selected.publish_prefix_boundary();
        let outcome = episode_replay::run_prefix_and_continue(
            self.port,
            self.runner,
            prefix,
            self.continuation,
            &mut publish_boundary,
        )?;
        if matches!(
            outcome,
            episode_replay::ReplayOutcome::Terminal {
                stage: EpisodeStage::Unknown,
                ..
            }
        ) {
            return Err(String::from(
                "branch continuation ended at an unknown gameplay stage",
            ));
        }
        Ok(outcome)
    }
}

fn terminal_result(
    report: sts2_harness::EpisodeRunReport,
) -> Result<(EpisodeStage, sts2_harness::EpisodeObservation), String> {
    if report.terminal_stage() == EpisodeStage::Unknown {
        return Err(String::from(
            "exact gameplay continuation ended at an unknown stage",
        ));
    }
    Ok((report.terminal_stage(), report.final_observation().clone()))
}
