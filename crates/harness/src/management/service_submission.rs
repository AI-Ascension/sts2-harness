// SPDX-License-Identifier: MIT

use super::super::auth::AuthContext;
use super::super::context_owner::{ContextBindingRequest, ContextOwnerPort};
use super::super::contract::{
    CleanupState, RunRequest, RunSubmissionResponse, WorkflowRunStatus, digest_value,
};
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
    if is_live_profile(&request.profile) {
        let owner = service.context_owner_port();
        if !owner.is_available() {
            return Err(ManagementError::unavailable(
                "context_owner_unavailable",
                "live workflow admission requires an attached authoritative context owner",
            ));
        }
        admit_context_owner(owner, actor, &request, &definition_digest)?;
    }
    let binding = service.revalidate_target_admission(actor, &request, &definition_digest)?;
    let reservation = RunReservation::new(
        std::sync::Arc::clone(&service.store),
        request.request_id.clone(),
        request_digest.clone(),
        definition_digest.clone(),
        binding.clone(),
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

/// Consults and binds the actor-scoped context owner before the target
/// authority or execution port can cross an effect boundary. The binding is
/// intentionally a bounded admission fact; the owner remains authoritative
/// for context bytes and subsequent control receipts.
fn admit_context_owner(
    owner: &dyn ContextOwnerPort,
    actor: &AuthContext,
    request: &RunRequest,
    definition_digest: &str,
) -> Result<(), ManagementError> {
    let catalog = owner.catalog(actor)?;
    catalog.validate()?;
    let definition = request.definition.as_ref().ok_or_else(|| {
        ManagementError::unavailable(
            "context_owner_binding_unavailable",
            "live context admission requires an inline workflow definition",
        )
    })?;
    let parsed = super::super::workflow_ports::parse_definition(definition)?;
    let (graph_id, node_id, node_kind, context_ref) = first_context_node(&parsed)?;
    let descriptor = catalog.descriptor_for(context_ref, node_kind)?;
    if !descriptor.grants.metadata_read {
        return Err(ManagementError::capability(
            "context_binding_metadata_unavailable",
            "context owner catalog does not grant metadata access for this node",
        ));
    }
    let binding_request = ContextBindingRequest {
        workflow_run_id: live_run_id(request, definition_digest)?,
        definition_digest: definition_digest.to_owned(),
        instance_id: request.instance_id.clone(),
        graph_id: graph_id.to_owned(),
        node_id: node_id.to_owned(),
        node_execution_id: "live.node.1".to_owned(),
        node_kind: node_kind.to_owned(),
        context_ref: context_ref.to_owned(),
        binding_id: descriptor.binding_id.clone(),
        binding_version: descriptor.version,
        binding_digest: descriptor.digest.clone(),
    };
    binding_request.validate()?;
    let binding = owner.bind(actor, &binding_request)?;
    binding.validate_for_request(&binding_request)?;
    if binding.owner_id != catalog.owner_id || binding.owner_version != catalog.owner_version {
        return Err(ManagementError::conflict(
            "context_owner_binding_foreign",
            "context owner binding was issued by a different catalog owner",
        ));
    }
    if binding.binding_id != descriptor.binding_id
        || binding.binding_version != descriptor.version
        || binding.binding_digest != descriptor.digest
        || binding.context_ref != descriptor.context_ref
    {
        return Err(ManagementError::conflict(
            "context_owner_binding_descriptor_mismatch",
            "context owner binding does not match the actor-scoped catalog descriptor",
        ));
    }
    Ok(())
}

fn first_context_node(
    definition: &WorkflowDefinition,
) -> Result<(&str, &str, &str, &str), ManagementError> {
    let graph = definition
        .graphs
        .iter()
        .find(|graph| graph.id == definition.entry_graph)
        .ok_or_else(|| {
            ManagementError::invalid(
                "context_owner_graph_missing",
                "workflow entry graph is missing from the admitted definition",
            )
        })?;
    graph
        .nodes
        .iter()
        .find_map(|node| match node {
            NodeDefinition::Analyze { id, config } => Some((
                graph.id.as_str(),
                id.as_str(),
                "analyze",
                config.context_ref.as_str(),
            )),
            NodeDefinition::Decide { id, config } => Some((
                graph.id.as_str(),
                id.as_str(),
                "decide",
                config.context_ref.as_str(),
            )),
            _ => None,
        })
        .ok_or_else(|| {
            ManagementError::capability(
                "context_binding_unsupported",
                "live workflow has no supported context-bound node",
            )
        })
}

fn live_run_id(request: &RunRequest, definition_digest: &str) -> Result<String, ManagementError> {
    let digest = digest_value(&serde_json::json!({
        "request_id": request.request_id,
        "instance_id": request.instance_id,
        "definition_digest": definition_digest,
    }))?;
    Ok(format!("run.live.{}", &digest[..32]))
}
