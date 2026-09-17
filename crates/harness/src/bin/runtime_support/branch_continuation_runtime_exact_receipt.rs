// SPDX-License-Identifier: MIT

impl SelectedBranchContinuation {
    /// Persists the independently verified destination receipt before publishing the branch.
    pub(crate) fn publish_exact_restore_receipt(
        &mut self,
        receipt_bytes: &[u8],
    ) -> Result<(), String> {
        if self.current_status()? != DurableBranchStatus::Restoring {
            return Err(String::from(
                "exact-restore receipt arrived outside the restoring state",
            ));
        }
        let artifact_store = ExactArtifactStore::new(&self.artifact_store_path);
        let receipt_digest = artifact_store
            .stage_blob(receipt_bytes)
            .map_err(|error| format!("cannot retain exact-restore receipt: {error}"))?;
        let reference = sts2_harness::BranchArtifactReference {
            artifact_id: receipt_digest.as_str().to_owned(),
            role: BranchArtifactRole::ContextSnapshot,
        };
        let attached = self
            .store
            .attach_artifact(
                &operation_suffix(&self.operation_id, "receipt-artifact"),
                &self.admission.branch.experiment_id,
                &self.admission.branch.branch_id,
                self.metadata_revision,
                reference,
            )
            .map_err(|error| format!("cannot attach exact-restore receipt: {error}"))?;
        self.metadata_revision = attached.metadata_revision;
        self.admission.branch = attached;
        let assured = self
            .store
            .set_assurance(
                &operation_suffix(&self.operation_id, "restore-assurance"),
                &self.admission.branch.experiment_id,
                &self.admission.branch.branch_id,
                self.metadata_revision,
                BranchAssurance::ExactRestoreReceipt,
            )
            .map_err(|error| format!("cannot persist exact-restore assurance: {error}"))?;
        self.metadata_revision = assured.metadata_revision;
        self.admission.branch = assured;
        let ready = self
            .store
            .transition(
                &operation_suffix(&self.operation_id, "restore-ready"),
                &self.admission.branch.experiment_id,
                &self.admission.branch.branch_id,
                self.metadata_revision,
                DurableBranchStatus::Ready,
            )
            .map_err(|error| format!("cannot publish exact-restore receipt: {error}"))?;
        self.metadata_revision = ready.metadata_revision;
        self.admission.branch = ready;
        if let Some(claim) = self.owner_claim.as_ref()
            && claim.state == sts2_harness::BranchContinuationClaimState::Claimed
        {
            let verified = self
                .store
                .transition_continuation_claim(
                    &claim.operation_id,
                    sts2_harness::BranchContinuationClaimState::Claimed,
                    sts2_harness::BranchContinuationClaimState::BoundaryVerified,
                )
                .map_err(|error| {
                    format!("cannot persist exact-restore owner boundary: {error}")
                })?;
            self.owner_claim = Some(verified);
        }
        // Publish Running only after the owner boundary is durable. A crash
        // before this point leaves a non-playable Ready branch; a crash after
        // it always leaves the resume claim eligible for the same owner.
        let running = self
            .store
            .transition(
                &operation_suffix(&self.operation_id, "restore-running"),
                &self.admission.branch.experiment_id,
                &self.admission.branch.branch_id,
                self.metadata_revision,
                DurableBranchStatus::Running,
            )
            .map_err(|error| format!("cannot publish exact restored branch: {error}"))?;
        self.metadata_revision = running.metadata_revision;
        self.admission.branch = running;
        Ok(())
    }
}
