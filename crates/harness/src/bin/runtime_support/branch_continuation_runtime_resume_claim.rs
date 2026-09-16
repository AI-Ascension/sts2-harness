// SPDX-License-Identifier: MIT

impl SelectedBranchContinuation {
    /// Claims the persisted resume phase while this selection holds the process-level lock.
    pub(crate) fn claim_resume(&mut self) -> Result<(), String> {
        if !self.resuming {
            return Err(String::from(
                "resume claim does not match the selected branch mode",
            ));
        }
        let claim = self
            .owner_claim
            .as_ref()
            .ok_or_else(|| String::from("selected branch has no owner claim"))?;
        if !matches!(
            claim.state,
            BranchContinuationClaimState::BoundaryVerified | BranchContinuationClaimState::Resuming
        ) {
            return Err(String::from(
                "selected branch owner claim is not eligible for resume",
            ));
        }
        let updated = self
            .store
            .transition_continuation_claim(
                &claim.operation_id,
                claim.state,
                BranchContinuationClaimState::Resuming,
            )
            .map_err(|error| format!("cannot claim selected branch resume: {error}"))?;
        self.owner_claim = Some(updated);
        Ok(())
    }

    /// Claims the replay attempt before the first runtime effect using the admitted CAS revision.
    pub(crate) fn claim_prefix_replay(&mut self) -> Result<(), String> {
        if !matches!(
            self.admission.strategy,
            BranchContinuationStrategyPlan::PrefixReplay { .. }
        ) {
            return Err(String::from(
                "prefix replay claim does not match the selected branch strategy",
            ));
        }
        let claimed = self
            .store
            .transition(
                &operation_suffix(&self.operation_id, "claim"),
                &self.admission.branch.experiment_id,
                &self.admission.branch.branch_id,
                self.metadata_revision,
                DurableBranchStatus::Replaying,
            )
            .map_err(|error| format!("cannot claim prefix replay branch: {error}"))?;
        self.metadata_revision = claimed.metadata_revision;
        Ok(())
    }
}
