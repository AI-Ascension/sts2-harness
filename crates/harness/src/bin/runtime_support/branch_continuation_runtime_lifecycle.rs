// SPDX-License-Identifier: MIT

impl SelectedBranchContinuation {
    pub(crate) fn prepare_owner_claim(
        &mut self,
    ) -> Result<sts2_harness::BranchContinuationClaim, String> {
        let claim = match self.owner_claim.clone() {
            Some(claim) => claim,
            None => self
                .store
                .prepare_continuation_claim(
                    &self.admission.branch.experiment_id,
                    &self.admission.branch.branch_id,
                )
                .map_err(|error| {
                    format!("cannot persist selected branch owner-claim intent: {error}")
                })?,
        };
        self.owner_claim = Some(claim.clone());
        Ok(claim)
    }

    pub(crate) fn exact_restore_owner(
        &self,
    ) -> Result<(String, serde_json::Value), String> {
        let claim = self
            .store
            .continuation_claim(
                &self.admission.branch.experiment_id,
                &self.admission.branch.branch_id,
            )
            .map_err(|error| format!("cannot read exact-restore owner claim: {error}"))?
            .ok_or_else(|| String::from("exact-restore owner claim is missing"))?;
        if self
            .owner_claim
            .as_ref()
            .is_none_or(|prepared| prepared.operation_id != claim.operation_id)
        {
            return Err(String::from(
                "exact-restore owner operation changed after preparation",
            ));
        }
        let owner_json = claim
            .owner_json
            .ok_or_else(|| String::from("exact-restore owner fence was not persisted"))?;
        let owner_digest = claim
            .owner_digest
            .ok_or_else(|| String::from("exact-restore owner digest was not persisted"))?;
        if sts2_harness::sha256_hex(owner_json.as_bytes()) != owner_digest {
            return Err(String::from(
                "exact-restore owner fence failed its integrity check",
            ));
        }
        let owner = super::gateway_json::parse(owner_json.as_bytes())
            .map_err(|_| String::from("exact-restore owner fence is invalid JSON"))?;
        Ok((claim.operation_id, owner))
    }

    pub(crate) const fn is_resuming(&self) -> bool {
        self.resuming
    }

    pub(crate) fn publish_prefix_boundary(&mut self) -> Result<(), String> {
        if self.current_status()? != sts2_harness::DurableBranchStatus::Replaying {
            return Err(String::from(
                "prefix replay boundary arrived outside the claimed replay state",
            ));
        }
        let assured = self
            .store
            .set_assurance(
                &operation_suffix(&self.operation_id, "assurance"),
                &self.admission.branch.experiment_id,
                &self.admission.branch.branch_id,
                self.metadata_revision,
                sts2_harness::BranchAssurance::PrefixReplayBoundary,
            )
            .map_err(|error| format!("cannot persist prefix replay evidence: {error}"))?;
        self.metadata_revision = assured.metadata_revision;
        let ready = self.store.transition(
            &operation_suffix(&self.operation_id, "ready"),
            &self.admission.branch.experiment_id,
            &self.admission.branch.branch_id,
            self.metadata_revision,
            sts2_harness::DurableBranchStatus::Ready,
        ).map_err(|error| format!("cannot publish verified branch boundary: {error}"))?;
        self.metadata_revision = ready.metadata_revision;
        let running = self.store.transition(
            &operation_suffix(&self.operation_id, "running"),
            &self.admission.branch.experiment_id,
            &self.admission.branch.branch_id,
            self.metadata_revision,
            sts2_harness::DurableBranchStatus::Running,
        ).map_err(|error| format!("cannot admit branch continuation: {error}"))?;
        self.metadata_revision = running.metadata_revision;
        if let Some(claim) = self.owner_claim.as_ref()
            && claim.state == sts2_harness::BranchContinuationClaimState::Claimed
        {
            self.store.transition_continuation_claim(
                &claim.operation_id,
                sts2_harness::BranchContinuationClaimState::Claimed,
                sts2_harness::BranchContinuationClaimState::BoundaryVerified,
            ).map_err(|error| format!("cannot persist verified owner boundary: {error}"))?;
        }
        Ok(())
    }

    pub(crate) fn mark_failed(&mut self, reason: &str) -> Result<(), String> {
        let status = self.current_status()?;
        let target = match status {
            sts2_harness::DurableBranchStatus::Restoring
            | sts2_harness::DurableBranchStatus::Replaying => sts2_harness::DurableBranchStatus::Failed,
            sts2_harness::DurableBranchStatus::Running => sts2_harness::DurableBranchStatus::Unknown,
            _ => return Ok(()),
        };
        let updated = self.store.transition(
            &operation_suffix(&self.operation_id, "failed"),
            &self.admission.branch.experiment_id,
            &self.admission.branch.branch_id,
            self.metadata_revision,
            target,
        ).map_err(|error| format!("cannot record branch continuation {reason}: {error}"))?;
        self.metadata_revision = updated.metadata_revision;
        Ok(())
    }

    pub(crate) fn mark_unknown(&mut self, reason: &str) -> Result<(), String> {
        let status = self.current_status()?;
        if status != sts2_harness::DurableBranchStatus::Restoring {
            return Err(String::from(
                "exact-restore uncertainty arrived outside the restoring state",
            ));
        }
        let updated = self.store.transition(
            &operation_suffix(&self.operation_id, "unknown"),
            &self.admission.branch.experiment_id,
            &self.admission.branch.branch_id,
            self.metadata_revision,
            sts2_harness::DurableBranchStatus::Unknown,
        ).map_err(|error| format!("cannot record branch continuation {reason}: {error}"))?;
        self.metadata_revision = updated.metadata_revision;
        self.admission.branch = updated;
        Ok(())
    }

    pub(crate) fn claim_exact_restore(&mut self) -> Result<(), String> {
        if !self.is_exact_restore() || self.current_status()? != sts2_harness::DurableBranchStatus::Ready {
            return Err(String::from(
                "exact-restore claim does not match a ready exact branch",
            ));
        }
        let restoring = self.store.transition(
            &operation_suffix(&self.operation_id, "claim-restore"),
            &self.admission.branch.experiment_id,
            &self.admission.branch.branch_id,
            self.metadata_revision,
            sts2_harness::DurableBranchStatus::Restoring,
        ).map_err(|error| format!("cannot claim exact-restore branch: {error}"))?;
        self.metadata_revision = restoring.metadata_revision;
        self.admission.branch = restoring;
        Ok(())
    }

    pub(crate) fn complete(&mut self) -> Result<(), String> {
        let completed = self.store.transition(
            &operation_suffix(&self.operation_id, "complete"),
            &self.admission.branch.experiment_id,
            &self.admission.branch.branch_id,
            self.metadata_revision,
            sts2_harness::DurableBranchStatus::Completed,
        ).map_err(|error| format!("cannot complete durable branch continuation: {error}"))?;
        self.metadata_revision = completed.metadata_revision;
        Ok(())
    }

    fn current_status(&self) -> Result<sts2_harness::DurableBranchStatus, String> {
        self.store
            .get(
                &self.admission.branch.experiment_id,
                &self.admission.branch.branch_id,
            )
            .map_err(|error| format!("cannot read branch continuation state: {error}"))?
            .map(|branch| branch.status)
            .ok_or_else(|| String::from("selected branch disappeared from its store"))
    }
}
