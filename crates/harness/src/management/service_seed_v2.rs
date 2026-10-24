// SPDX-License-Identifier: MIT

use super::super::auth::AuthContext;
use super::super::contract::{digest_value, schema_is, validate_identifier};
use super::super::contract_seed_v2::{
    SeedBindingReadbackV2, SeedBindingStateV2, SeededRunSubmissionResponseV2,
    WORKFLOW_RUN_REQUEST_V2_SCHEMA, WorkflowRunRequestV2,
};
use super::super::seed_v2_crypto::{
    actor_digest, candidate_record, request_digest, verify_candidate, verify_stored_record,
};
use super::super::store::{SeedBindingLookup, SeedBindingRecord};
use super::inference_profile_ops::{admit_inference_profiles, bind_inference_provenance};
use super::seed_v2_support::{
    corrupt_seed_record, response, seed_binding_not_found, seed_key_error, seed_request_error,
    submission_conflict, validate_definition,
};
use super::support::{authorize, enforce_live_profile, validate_profile};
use super::target_admission::{
    bind_snapshot_admission, is_live_profile, validate_run_target_admission,
};
use super::{ManagementError, ManagementService, RunReservation};

impl ManagementService {
    /// Submit the additive v2 request through the same live execution port as
    /// v1. The execution adapter persists the candidate before any session is
    /// opened and explicitly refuses when it cannot prove that ordering.
    pub fn submit_seeded_run_v2(
        &self,
        actor: &AuthContext,
        request: WorkflowRunRequestV2,
    ) -> Result<SeededRunSubmissionResponseV2, ManagementError> {
        submit(self, actor, request)
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

fn submit(
    service: &ManagementService,
    actor: &AuthContext,
    request: WorkflowRunRequestV2,
) -> Result<SeededRunSubmissionResponseV2, ManagementError> {
    authorize(actor, "workflow:control", None)?;
    schema_is(&request.schema_version, WORKFLOW_RUN_REQUEST_V2_SCHEMA)?;
    request.validate_seed().map_err(seed_request_error)?;
    validate_identifier("request_id", &request.request_id)?;
    validate_identifier("instance_id", &request.instance_id)?;
    validate_profile(&request.profile)?;
    let execution_request = request.clone().into_execution_request();
    validate_run_target_admission(&execution_request)?;
    if request.definition.is_some() == request.artifact_id.is_some() {
        return Err(ManagementError::invalid(
            "run_source_count",
            "exactly one definition or artifact_id is required",
        ));
    }
    if let Some(artifact_id) = request.artifact_id.as_deref() {
        validate_identifier("artifact_id", artifact_id)?;
    }
    if is_live_profile(&request.profile) && !actor.can("workflow:live") {
        return Err(ManagementError::forbidden(
            "live_scope_required",
            "live workflow submission requires the workflow:live scope",
        ));
    }

    let request_digest = request_digest(&request, &actor.subject).map_err(seed_key_error)?;
    let actor_digest = actor_digest(&actor.subject).map_err(seed_key_error)?;
    match service
        .store
        .lookup_seed_binding(&request.request_id, &actor_digest, &request_digest)?
    {
        SeedBindingLookup::Existing(record) => {
            return existing_submission(service, actor, &request, &request_digest, *record);
        }
        SeedBindingLookup::Conflict => return Err(submission_conflict()),
        SeedBindingLookup::Missing => {}
    }

    enforce_live_profile(&service.capabilities, actor, &request.profile)?;
    let definition_digest = validate_definition(service, &request)?;
    let binding = service
        .revalidate_target_admission(actor, &execution_request, &definition_digest)?
        .ok_or_else(|| {
            ManagementError::conflict(
                "seed_admission_required",
                "v2 seed submission requires an owner-revalidated target admission",
            )
        })?;
    let resolved = if is_live_profile(&request.profile) {
        admit_inference_profiles(service, actor, &execution_request, &binding)?
    } else {
        None
    };
    let binding = bind_inference_provenance(Some(binding), resolved.as_ref()).ok_or_else(|| {
        ManagementError::conflict(
            "seed_admission_required",
            "v2 seed submission requires an owner-revalidated target admission",
        )
    })?;
    let mut execution_request = request.clone().into_execution_request();
    execution_request.admission = Some(binding.clone());
    let workflow_run_id = super::super::live_run_id(&execution_request, &definition_digest)?;
    let seed_binding = candidate_record(
        &request,
        actor,
        &request_digest,
        &workflow_run_id,
        &binding,
        service.seed_derivation_keys(),
    )
    .map_err(seed_key_error)?;
    let reservation = RunReservation::for_seed_candidate(
        std::sync::Arc::clone(&service.store),
        request.request_id.clone(),
        request_digest.clone(),
        definition_digest.clone(),
        Some(binding.clone()),
        seed_binding,
    );

    let admission = match service.execution.prepare_seed_candidate_with_reservation(
        &execution_request,
        actor,
        &definition_digest,
        Some(&binding),
        &reservation,
    ) {
        Ok(admission) => admission,
        Err(original) => {
            // Another process may have won the immediate transaction after our
            // initial miss. A retry returns only the exact actor/body winner,
            // verified with its persisted key version and admitted snapshot.
            return match service.store.lookup_seed_binding(
                &request.request_id,
                &actor_digest,
                &request_digest,
            )? {
                SeedBindingLookup::Existing(record) => {
                    existing_submission(service, actor, &request, &request_digest, *record)
                }
                SeedBindingLookup::Conflict => Err(submission_conflict()),
                SeedBindingLookup::Missing => Err(original),
            };
        }
    };
    let mut admission = admission;
    admission.snapshot = bind_snapshot_admission(admission.snapshot, Some(&binding))?;
    super::support::verify_admission(&admission, &definition_digest)?;
    service.store.update_run_snapshot(
        &request.request_id,
        &request_digest,
        admission.snapshot.clone(),
    )?;

    let stored = service.store.mark_seed_binding_awaiting_host_context(
        &workflow_run_id,
        &actor_digest,
        &request_digest,
    )?;
    let readback = service
        .store
        .read_seed_binding(&workflow_run_id)?
        .ok_or_else(|| corrupt_seed_record("seed binding disappeared after state update"))?;
    if stored.record() != readback.record() {
        return Err(corrupt_seed_record(
            "seed binding changed during strong readback",
        ));
    }
    let readback = readback.record();
    if !verify_stored_record(readback, actor, service.seed_derivation_keys())
        .map_err(seed_key_error)?
    {
        return Err(corrupt_seed_record(
            "persisted seed candidate failed pinned-key verification",
        ));
    }
    Ok(response(&admission.snapshot, readback.readback()))
}

fn existing_submission(
    service: &ManagementService,
    actor: &AuthContext,
    request: &WorkflowRunRequestV2,
    request_digest: &str,
    record: SeedBindingRecord,
) -> Result<SeededRunSubmissionResponseV2, ManagementError> {
    let record = record.record();
    let workflow_run_id = &record.workflow_run_id;
    authorize(actor, "workflow:control", Some(workflow_run_id))?;
    let snapshot = service
        .store
        .get_run(workflow_run_id)?
        .ok_or_else(|| corrupt_seed_record("seed binding has no workflow snapshot"))?;
    if snapshot.workflow_run_id != *workflow_run_id {
        return Err(corrupt_seed_record(
            "workflow snapshot identity does not match its seed binding",
        ));
    }
    if !verify_candidate(
        record,
        request,
        actor,
        request_digest,
        workflow_run_id,
        service.seed_derivation_keys(),
    )
    .map_err(seed_key_error)?
    {
        return Err(corrupt_seed_record(
            "stored seed binding failed its owner and key verification",
        ));
    }
    let stored = if record.state == SeedBindingStateV2::CandidatePersisted {
        service.store.mark_seed_binding_awaiting_host_context(
            workflow_run_id,
            &record.actor_digest,
            request_digest,
        )?
    } else {
        SeedBindingRecord::new(record.clone())
    };
    let readback = service
        .store
        .read_seed_binding(workflow_run_id)?
        .ok_or_else(|| corrupt_seed_record("seed binding disappeared during replay"))?;
    if stored.record() != readback.record() {
        return Err(corrupt_seed_record(
            "seed binding changed during replay readback",
        ));
    }
    let readback = readback.record();
    if !verify_candidate(
        readback,
        request,
        actor,
        request_digest,
        workflow_run_id,
        service.seed_derivation_keys(),
    )
    .map_err(seed_key_error)?
    {
        return Err(corrupt_seed_record(
            "replayed seed candidate failed pinned-key verification",
        ));
    }
    Ok(response(&snapshot, readback.readback()))
}
