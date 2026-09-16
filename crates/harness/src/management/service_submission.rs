// SPDX-License-Identifier: MIT

use super::super::auth::AuthContext;
use super::super::context_owner::ContextOwnerPort;
use super::super::contract::{CleanupState, RunRequest, RunSubmissionResponse, WorkflowRunStatus};
use super::support::{run_submission_response, verify_admission};
use super::target_admission::{bind_snapshot_admission, is_live_profile};
use super::{ManagementError, ManagementService, RunReservation};
use crate::workflow::{NodeDefinition, WorkflowDefinition};

pub(super) fn submit_run(
    service: &ManagementService,
    actor: &AuthContext,
    request: RunRequest,
    request_digest: String,
    definition_digest: String,
) -> Result<RunSubmissionResponse, ManagementError> {
    // A live invocation must be bound to the authoritative context owner before
    // any reservation, session open, or launch. Fail closed when it is absent.
    let context_control_limits = if is_live_profile(&request.profile) {
        let owner = service.context_owner_port();
        if !owner.is_available() {
            return Err(ManagementError::unavailable(
                "context_owner_unavailable",
                "live workflow admission requires an attached authoritative context owner",
            ));
        }
        Some(admit_context_owner(
            owner,
            actor,
            &request,
            &definition_digest,
        )?)
    } else {
        None
    };
    let binding = service.revalidate_target_admission(actor, &request, &definition_digest)?;
    let reservation = RunReservation::new(
        std::sync::Arc::clone(&service.store),
        request.request_id.clone(),
        request_digest.clone(),
        definition_digest.clone(),
        binding.clone(),
        context_control_limits,
    );
    let execution_result = service.execution.submit_admitted_with_reservation(
        &request,
        actor,
        &definition_digest,
        binding.as_ref(),
        &reservation,
    );
    let mut admission = match execution_result {
        Ok(admission) => admission,
        Err(error) => {
            return Err(persist_reserved_failure(
                service,
                &reservation,
                &request,
                &request_digest,
                error,
            ));
        }
    };
    let bound_snapshot = match bind_snapshot_admission(admission.snapshot.clone(), binding.as_ref())
    {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return Err(persist_reserved_failure(
                service,
                &reservation,
                &request,
                &request_digest,
                error,
            ));
        }
    };
    admission.snapshot = bound_snapshot;
    if let Err(error) = verify_admission(&admission, &definition_digest) {
        return Err(persist_reserved_failure(
            service,
            &reservation,
            &request,
            &request_digest,
            error,
        ));
    }
    if is_live_profile(&request.profile)
        && let Err(error) = service.store.update_run_snapshot(
            &request.request_id,
            &request_digest,
            admission.snapshot.clone(),
        )
    {
        return Err(persist_reserved_failure(
            service,
            &reservation,
            &request,
            &request_digest,
            error.into(),
        ));
    }
    Ok(run_submission_response(&admission.snapshot))
}

fn persist_reserved_failure(
    service: &ManagementService,
    reservation: &RunReservation,
    request: &RunRequest,
    request_digest: &str,
    original: ManagementError,
) -> ManagementError {
    let snapshot = match reservation.take_snapshot() {
        Ok(snapshot) => snapshot,
        Err(_) => {
            return ManagementError::store(
                "reservation_failure_persist",
                format!(
                    "submission failed ({}), and its reservation state could not be read",
                    original.code
                ),
            );
        }
    };
    let Some(mut snapshot) = snapshot else {
        return original;
    };
    let cleanup_error = service
        .execution
        .abort_submission(&snapshot.workflow_run_id)
        .err();
    snapshot.status = WorkflowRunStatus::NeedsOperator;
    snapshot.cleanup = CleanupState::NeedsOperator;
    match service
        .store
        .update_run_snapshot(&request.request_id, request_digest, snapshot)
    {
        Ok(()) => cleanup_error.map_or(original.clone(), |error| {
            ManagementError::unavailable(
                "live_submission_cleanup_failed",
                format!(
                    "submission failed ({}), and live cleanup failed ({})",
                    original.code, error.code
                ),
            )
        }),
        Err(persist_error) => ManagementError::store(
            "reservation_failure_persist",
            format!(
                "submission failed ({}), and its reservation could not be marked for recovery ({}{})",
                original.code,
                persist_error.code,
                cleanup_error
                    .as_ref()
                    .map_or(String::new(), |error| format!(
                        ", cleanup failed ({})",
                        error.code
                    ))
            ),
        ),
    }
}

/// Gates live admission on the authoritative context owner.
///
/// This is a bounded pre-effect support check: the owner must be available and
/// its catalog must advertise a usable, metadata-readable binding for every
/// context-bound node in the definition. The per-invocation binding itself is
/// established at dispatch (`bind_dispatch_context`), where the runtime-allocated
/// `node_execution_id` is known, so admission can never accept a binding for an
/// invocation the run does not execute.
fn admit_context_owner(
    owner: &dyn ContextOwnerPort,
    actor: &AuthContext,
    request: &RunRequest,
    _definition_digest: &str,
) -> Result<super::super::context_owner::ContextOwnerControlLimits, ManagementError> {
    let catalog = owner.catalog(actor)?;
    catalog.validate()?;
    let definition = request.definition.as_ref().ok_or_else(|| {
        ManagementError::unavailable(
            "context_owner_binding_unavailable",
            "live context admission requires an inline workflow definition",
        )
    })?;
    let parsed = super::super::workflow_ports::parse_definition(definition)?;
    validate_context_nodes(&parsed, &catalog)
}

fn validate_context_nodes(
    definition: &WorkflowDefinition,
    catalog: &super::super::context_owner::ContextBindingCatalog,
) -> Result<super::super::context_owner::ContextOwnerControlLimits, ManagementError> {
    let mut descriptors = Vec::new();
    for graph in &definition.graphs {
        for node in &graph.nodes {
            let Some((_, node_kind, context_ref)) = context_node_parts(node) else {
                continue;
            };
            let descriptor = catalog.descriptor_for(context_ref, node_kind)?;
            if !descriptor.grants.metadata_read {
                return Err(ManagementError::capability(
                    "context_binding_metadata_unavailable",
                    "context owner catalog does not grant metadata access for this node",
                ));
            }
            descriptors.push(descriptor);
        }
    }
    super::super::context_owner::ContextOwnerControlLimits::from_descriptors(catalog, &descriptors)
}

fn context_node_parts(node: &NodeDefinition) -> Option<(&str, &str, &str)> {
    match node {
        NodeDefinition::Analyze { id, config } => {
            Some((id.as_str(), "analyze", config.context_ref.as_str()))
        }
        NodeDefinition::Decide { id, config } => {
            Some((id.as_str(), "decide", config.context_ref.as_str()))
        }
        _ => None,
    }
}
