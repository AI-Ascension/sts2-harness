// SPDX-License-Identifier: MIT

use super::super::super::auth::AuthContext;
use super::super::super::contract::{schema_is, validate_identifier};
use super::super::super::contract_seed_v2::{
    SeedModeV2, SeededRunSubmissionResponseV2, WORKFLOW_RUN_REQUEST_V2_SCHEMA, WorkflowRunRequestV2,
};
use super::super::super::seed_v2_crypto::{
    actor_digest, candidate_record, prepare_seed_operation, request_digest, verify_stored_record,
};
use super::super::super::store::{SeedBindingLookup, SeedOperationLookup, SeedOperationRecord};
use super::super::inference_profile_ops::{admit_inference_profiles, bind_inference_provenance};
use super::super::seed_v2_support::{
    corrupt_seed_record, derive_from_pinned_operation, existing_submission, response,
    same_operation_request, seed_key_error, seed_request_error, submission_conflict,
    validate_definition,
};
use super::super::support::{authorize, enforce_live_profile, validate_profile, verify_admission};
use super::super::target_admission::{
    bind_snapshot_admission, is_live_profile, validate_run_target_admission,
};
use super::super::{ManagementError, ManagementService, RunReservation};

pub(super) fn submit(
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
    let mut execution_request = request.clone().into_execution_request();
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
    let definition_digest = validate_definition(service, &request)?;
    let prepared_operation = if request.seed.mode == SeedModeV2::DeriveOnce {
        if !service.store.supports_seed_operation_reservations() {
            return Err(ManagementError::unavailable(
                "seed_operation_store_unavailable",
                "workflow store cannot durably pin derive-once key selection",
            ));
        }
        match service.store.lookup_seed_operation(
            &request.request_id,
            &actor_digest,
            &request_digest,
        )? {
            SeedOperationLookup::Prepared(operation) => Some((*operation).into_record()),
            SeedOperationLookup::CandidatePersisted(record) => {
                return existing_submission(service, actor, &request, &request_digest, *record);
            }
            SeedOperationLookup::Conflict => return Err(submission_conflict()),
            SeedOperationLookup::Missing => None,
        }
    } else {
        match service.store.lookup_seed_binding(
            &request.request_id,
            &actor_digest,
            &request_digest,
        )? {
            SeedBindingLookup::Existing(record) => {
                return existing_submission(service, actor, &request, &request_digest, *record);
            }
            SeedBindingLookup::Conflict => return Err(submission_conflict()),
            SeedBindingLookup::Missing => None,
        }
    };

    let (binding, workflow_run_id, seed_binding, seed_operation, execution_request) =
        if let Some(operation) = prepared_operation {
            let binding = operation.admitted_configuration.clone();
            let mut execution_request = request.clone().into_execution_request();
            execution_request.admission = Some(binding.clone());
            let workflow_run_id =
                super::super::super::live_run_id(&execution_request, &definition_digest)?;
            if workflow_run_id != operation.workflow_run_id {
                return Err(corrupt_seed_record(
                    "prepared seed operation run identity no longer matches its request",
                ));
            }
            let seed_binding = derive_from_pinned_operation(
                service,
                &operation,
                actor,
                &request,
                &request_digest,
            )?;
            (
                binding,
                workflow_run_id,
                seed_binding,
                Some(SeedOperationRecord::new(operation)),
                execution_request,
            )
        } else {
            enforce_live_profile(&service.capabilities, actor, &request.profile)?;
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
            let binding =
                bind_inference_provenance(Some(binding), resolved.as_ref()).ok_or_else(|| {
                    ManagementError::conflict(
                        "seed_admission_required",
                        "v2 seed submission requires an owner-revalidated target admission",
                    )
                })?;
            execution_request.admission = Some(binding.clone());
            let workflow_run_id =
                super::super::super::live_run_id(&execution_request, &definition_digest)?;
            if request.seed.mode == SeedModeV2::DeriveOnce {
                let proposed = prepare_seed_operation(
                    &request,
                    actor,
                    &request_digest,
                    &workflow_run_id,
                    &binding,
                    service.seed_derivation_keys(),
                )
                .map_err(seed_key_error)?;
                let proposed_wrapper = SeedOperationRecord::new(proposed.clone());
                match service.store.reserve_seed_operation(proposed_wrapper)? {
                    SeedOperationLookup::Conflict => return Err(submission_conflict()),
                    SeedOperationLookup::CandidatePersisted(record) => {
                        return existing_submission(
                            service,
                            actor,
                            &request,
                            &request_digest,
                            *record,
                        );
                    }
                    SeedOperationLookup::Prepared(_) => {}
                    SeedOperationLookup::Missing => {
                        return Err(corrupt_seed_record(
                            "seed operation disappeared after arbitration",
                        ));
                    }
                }
                // Always perform a strong lookup after insert-or-win. A race
                // loser must use the committed winner even when its provisional
                // current-key identity differs.
                let stored = match service.store.lookup_seed_operation(
                    &request.request_id,
                    &actor_digest,
                    &request_digest,
                )? {
                    SeedOperationLookup::Prepared(record) => *record,
                    SeedOperationLookup::CandidatePersisted(record) => {
                        return existing_submission(
                            service,
                            actor,
                            &request,
                            &request_digest,
                            *record,
                        );
                    }
                    SeedOperationLookup::Conflict => return Err(submission_conflict()),
                    SeedOperationLookup::Missing => {
                        return Err(corrupt_seed_record(
                            "seed operation disappeared during strong readback",
                        ));
                    }
                };
                if !same_operation_request(&proposed, stored.record()) {
                    return Err(submission_conflict());
                }
                let seed_binding = derive_from_pinned_operation(
                    service,
                    stored.record(),
                    actor,
                    &request,
                    &request_digest,
                )?;
                (
                    binding,
                    workflow_run_id,
                    seed_binding,
                    Some(stored),
                    execution_request,
                )
            } else {
                let seed_binding = candidate_record(
                    &request,
                    actor,
                    &request_digest,
                    &workflow_run_id,
                    &binding,
                    service.seed_derivation_keys(),
                )
                .map_err(seed_key_error)?;
                (
                    binding,
                    workflow_run_id,
                    seed_binding,
                    None,
                    execution_request,
                )
            }
        };
    let reservation = RunReservation::for_seed_candidate(
        std::sync::Arc::clone(&service.store),
        request.request_id.clone(),
        request_digest.clone(),
        definition_digest.clone(),
        Some(binding.clone()),
        seed_operation,
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
    verify_admission(&admission, &definition_digest)?;
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
