// SPDX-License-Identifier: MIT

use super::super::auth::AuthContext;
use super::super::contract::validate_identifier;
use super::super::contract_seed_v2::{
    SeedBindingReadbackV2, SeededRunSubmissionResponseV2, WorkflowRunRequestV2,
};
use super::super::seed_v2_crypto::verify_stored_record;
use super::seed_v2_support::{corrupt_seed_record, seed_binding_not_found, seed_key_error};
use super::support::authorize;
use super::{ManagementError, ManagementService};

#[path = "service_seed_v2_submit.rs"]
mod submit;

impl ManagementService {
    /// Submit the additive v2 request through the same live execution port as
    /// v1. The execution adapter persists the candidate before any session is
    /// opened and explicitly refuses when it cannot prove that ordering.
    pub fn submit_seeded_run_v2(
        &self,
        actor: &AuthContext,
        request: WorkflowRunRequestV2,
    ) -> Result<SeededRunSubmissionResponseV2, ManagementError> {
        submit::submit(self, actor, request)
    }

    /// Read the immutable seed tuple through a run-scoped authenticated route.
    pub fn seed_binding_v2(
        &self,
        actor: &AuthContext,
        workflow_run_id: &str,
    ) -> Result<SeedBindingReadbackV2, ManagementError> {
        validate_identifier("workflow_run_id", workflow_run_id)?;
        authorize(actor, "workflow:read", Some(workflow_run_id))?;
        let record = self
            .store
            .read_seed_binding(workflow_run_id)?
            .ok_or_else(seed_binding_not_found)?;
        let record = record.record();
        if !record.visible_to(actor) {
            return Err(seed_binding_not_found());
        }
        let snapshot = self
            .store
            .get_run(workflow_run_id)?
            .ok_or_else(|| corrupt_seed_record("seed binding has no workflow snapshot"))?;
        if snapshot.workflow_run_id != workflow_run_id {
            return Err(corrupt_seed_record(
                "workflow snapshot identity does not match its seed binding",
            ));
        }
        if !verify_stored_record(record, actor, self.seed_derivation_keys())
            .map_err(seed_key_error)?
        {
            return Err(corrupt_seed_record(
                "stored seed binding failed its owner and key verification",
            ));
        }
        Ok(record.readback())
    }
}
