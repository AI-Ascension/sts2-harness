// SPDX-License-Identifier: MIT

impl SelectedBranchContinuation {
    pub(crate) fn verify_persisted_exact_receipt(
        &self,
        closure: &super::exact_restore::VerifiedClosure,
    ) -> Result<(), String> {
        let receipt = self
            .admission
            .branch
            .artifacts
            .iter()
            .filter(|artifact| artifact.role == sts2_harness::BranchArtifactRole::ContextSnapshot)
            .collect::<Vec<_>>();
        if receipt.len() != 1 {
            return Err(String::from(
                "running exact branch must retain exactly one destination receipt",
            ));
        }
        let digest = sts2_harness::BlobDigest::parse(&receipt[0].artifact_id)
            .map_err(|_| String::from("persisted exact-restore receipt is not a verified blob"))?;
        let artifacts = sts2_harness::ExactArtifactStore::new(&self.artifact_store_path);
        let bytes = artifacts
            .read_blob(&digest)
            .map_err(|error| format!("cannot read persisted exact-restore receipt: {error}"))?;
        self.verify_exact_restore_receipt_bytes(&bytes, closure)
    }

    /// Verifies receipt bytes against the durable claim, owner fence, and admitted closure.
    pub(crate) fn verify_exact_restore_receipt_bytes(
        &self,
        bytes: &[u8],
        closure: &super::exact_restore::VerifiedClosure,
    ) -> Result<(), String> {
        let receipt: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|error| format!("persisted exact-restore receipt is invalid JSON: {error}"))?;
        let receipt_frame = serde_json::json!({
            "contract": super::exact_restore::NEUTRAL_CONTRACT,
            "schema_digest": super::exact_restore::NEUTRAL_SCHEMA_DIGEST,
            "message_id": "00000000-0000-4000-8000-000000000010",
            "correlation_id": "00000000-0000-4000-8000-000000000011",
            "kind": "exact_restore_commit_response",
            "payload": {
                "operation_id": receipt["operation_id"],
                "result": "RESTORE_VERIFIED",
                "state": "RESTORE_VERIFIED",
                "expected_owner": receipt["destination_owner"],
                "request_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                "receipt": receipt
            }
        });
        if !super::exact_restore::valid_schema(
            super::exact_restore::neutral_validator()?,
            &receipt_frame,
        ) {
            return Err(String::from(
                "persisted exact-restore receipt does not satisfy its closed response schema",
            ));
        }
        self.verify_persisted_exact_receipt_revision(bytes)?;
        super::exact_restore::operation::verify_persisted_receipt(bytes, self, closure)
    }

    pub(crate) fn verify_persisted_exact_receipt_revision(
        &self,
        bytes: &[u8],
    ) -> Result<(), String> {
        let receipt: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|error| format!("persisted exact-restore receipt is invalid JSON: {error}"))?;
        let receipt_revision = receipt["branch"]["metadata_revision"]
            .as_u64()
            .ok_or_else(|| String::from("persisted exact-restore receipt omits branch revision"))?;
        let claim_operation = operation_suffix(&self.operation_id, "claim-restore");
        let mut cursor = 0_u64;
        let mut claim_event = None;
        for _ in 0..4096 {
            let page = self
                .store
                .events(&self.admission.branch.experiment_id, cursor, 256)
                .map_err(|error| format!("cannot read exact-restore claim history: {error}"))?;
            let page_empty = page.events.is_empty();
            claim_event = page.events.into_iter().find(|event| {
                event.branch_id == self.admission.branch.branch_id
                    && event.operation_id == claim_operation
                    && event.status == sts2_harness::DurableBranchStatus::Restoring
            });
            if claim_event.is_some() || page_empty || page.next_after_sequence <= cursor {
                break;
            }
            cursor = page.next_after_sequence;
            if page.newest_sequence.is_some_and(|newest| cursor >= newest) {
                break;
            }
        }
        let claim_event =
            claim_event.ok_or_else(|| String::from("exact-restore claim history is missing"))?;
        if claim_event.metadata_revision != receipt_revision {
            return Err(String::from(
                "persisted exact-restore receipt branch revision does not match its claim history",
            ));
        }
        Ok(())
    }

    pub(crate) fn is_exact_restore(&self) -> bool {
        matches!(
            self.admission.strategy,
            sts2_harness::BranchContinuationStrategyPlan::ExactRestore { .. }
        )
    }
}
