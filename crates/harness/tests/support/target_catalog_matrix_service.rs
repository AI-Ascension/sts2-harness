// SPDX-License-Identifier: MIT

//! Service composition for the multi-target admission matrix (#99): a
//! management service over the per-actor catalog double and the recording
//! execution port, plus the preflight/submit request builders and the SQLite
//! restart fixture shared by the matrix and durability suites.

#![allow(clippy::expect_used)]
#![allow(dead_code)]

use std::sync::Arc;

use sts2_harness::management::{
    AuthContext, CapabilityPort, MANAGEMENT_SCHEMA_VERSION, ManagementError, ManagementService,
    MemoryWorkflowStore, RunRequest, RunSnapshot, RunSubmissionResponse, RunTargetConfiguration,
    SqliteWorkflowStore, TARGET_ADMISSION_SCHEMA_VERSION, TargetAdmissionBinding,
    TargetAdmissionRequest, TargetCatalogResponse, WorkflowExecutionPort, WorkflowStore,
    digest_value,
};

use super::matrix::{
    Database, MatrixCatalog, MatrixDefinitionPort, RecordingExecutionPort, alpha_descriptor,
    beta_descriptor, persisted_admission, selection,
};
use super::support::{FakeContextOwner, definition};

pub(crate) type Outcome = Result<(), Box<dyn std::error::Error>>;

pub(crate) fn operator(subject: &str) -> AuthContext {
    AuthContext::new(subject, ["workflow:*".to_owned()]).expect("actor")
}

pub(crate) fn build_service(
    catalog: &Arc<MatrixCatalog>,
    port: &Arc<RecordingExecutionPort>,
    store: Arc<dyn WorkflowStore>,
) -> ManagementService {
    ManagementService::new(store)
        .with_capability_port(Arc::clone(catalog) as Arc<dyn CapabilityPort>)
        .with_definition_port(Arc::new(MatrixDefinitionPort))
        .with_context_owner_port(Arc::new(FakeContextOwner))
        .with_execution_port(Arc::clone(port) as Arc<dyn WorkflowExecutionPort>)
}

pub(crate) fn preflight(
    service: &ManagementService,
    actor: &AuthContext,
    request_id: &str,
    target: RunTargetConfiguration,
) -> Result<TargetAdmissionBinding, ManagementError> {
    let workflow_definition_digest = digest_value(&definition(false))?;
    service
        .preflight_target(
            actor,
            TargetAdmissionRequest {
                schema_version: TARGET_ADMISSION_SCHEMA_VERSION.to_owned(),
                request_id: request_id.to_owned(),
                workflow_definition_digest,
                target,
            },
        )
        .map(|response| response.admission)
}

pub(crate) fn run_request(request_id: &str, admission: TargetAdmissionBinding) -> RunRequest {
    RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: request_id.to_owned(),
        definition: Some(definition(false)),
        artifact_id: None,
        instance_id: admission.target.instance_id.clone(),
        profile: admission.target.execution_profile.clone(),
        admission: Some(admission),
    }
}

pub(crate) fn instance_ids(catalog: &TargetCatalogResponse) -> Vec<&str> {
    catalog
        .targets
        .iter()
        .map(|target| target.instance_id.as_str())
        .collect()
}

/// In-memory service whose operator sees both authorized fake targets.
pub(crate) fn two_target_fixture() -> (
    Arc<MatrixCatalog>,
    Arc<RecordingExecutionPort>,
    ManagementService,
) {
    let catalog = MatrixCatalog::new();
    catalog.serve(
        "operator",
        super::matrix::catalog(vec![alpha_descriptor(), beta_descriptor()]),
    );
    let port = RecordingExecutionPort::new();
    let service = build_service(&catalog, &port, Arc::new(MemoryWorkflowStore::new()));
    (catalog, port, service)
}

pub(crate) struct Restarted {
    pub(crate) catalog: Arc<MatrixCatalog>,
    pub(crate) request: RunRequest,
    pub(crate) first: RunSubmissionResponse,
    pub(crate) service: ManagementService,
    pub(crate) port: Arc<RecordingExecutionPort>,
    pub(crate) store: Arc<SqliteWorkflowStore>,
}

/// Preflights and submits alpha over SQLite, retries once on the same service
/// (lost response), then drops every handle and rebuilds the service over the
/// reopened database with a fresh recording port.
pub(crate) fn restart_fixture(
    database: &Database,
    request_id: &str,
) -> Result<Restarted, Box<dyn std::error::Error>> {
    let catalog = MatrixCatalog::new();
    catalog.serve(
        "operator",
        super::matrix::catalog(vec![alpha_descriptor(), beta_descriptor()]),
    );
    let full = operator("operator");
    let store = Arc::new(SqliteWorkflowStore::open(&database.path)?);
    let port = RecordingExecutionPort::new();
    let service = build_service(
        &catalog,
        &port,
        Arc::clone(&store) as Arc<dyn WorkflowStore>,
    );
    let alpha = preflight(
        &service,
        &full,
        request_id,
        selection(&alpha_descriptor(), "live.workflow.v1"),
    )?;
    let request = run_request(request_id, alpha.clone());
    let first = service.submit_run(&full, request.clone())?;
    assert_eq!(port.submissions(), 1);
    assert_eq!(service.submit_run(&full, request.clone())?, first);
    assert_eq!(
        port.submissions(),
        1,
        "lost-response retry must not resubmit"
    );
    assert_eq!(
        persisted_admission(store.as_ref(), &first.workflow_run_id),
        Some(alpha)
    );
    drop(service);
    assert_eq!(Arc::strong_count(&store), 1);
    drop(store);

    let store = Arc::new(SqliteWorkflowStore::open(&database.path)?);
    let port = RecordingExecutionPort::new();
    let service = build_service(
        &catalog,
        &port,
        Arc::clone(&store) as Arc<dyn WorkflowStore>,
    );
    Ok(Restarted {
        catalog,
        request,
        first,
        service,
        port,
        store,
    })
}

/// Current durable snapshot of the first submitted run, as served by status.
pub(crate) fn status_snapshot(
    restarted: &Restarted,
    actor: &AuthContext,
) -> Result<RunSnapshot, ManagementError> {
    restarted
        .service
        .status(actor, &restarted.first.workflow_run_id)
        .map(|status| status.run)
}
