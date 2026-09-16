// SPDX-License-Identifier: MIT

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use sts2_harness::management::{
    AuthContext, Budget, CapabilityPort, CleanupState, CommandApplication, CommandContext,
    DefinitionPort, Diagnostic, EVENT_SCHEMA_VERSION, EventClassification, EventPayload, EventType,
    ExecutionMode, GameOutcome, MANAGEMENT_SCHEMA_VERSION, ManagementClient, ManagementError,
    ManagementServer, ManagementService, RunAdmission, RunEvent, RunRequest, RunSnapshot,
    RunTargetConfiguration, ServerConfig, StaticAuthenticator, TargetAdmissionBinding,
    TargetAdmissionRequest, TargetAvailability, TargetCatalogResponse, TargetDescriptor,
    TargetPreflightResponse, ValidationResult, WorkflowExecutionPort, WorkflowRunStatus,
    decode_strict, digest_value,
};

#[path = "support/live_workflow_context_owner.rs"]
mod context_owner_double;
#[path = "support/live_workflow_definition.rs"]
mod live_workflow_definition;

use context_owner_double::FakeContextOwner;

struct DefinitionDouble;

impl DefinitionPort for DefinitionDouble {
    fn validate(
        &self,
        definition: &Value,
        _capabilities: &Value,
    ) -> Result<ValidationResult, ManagementError> {
        Ok(ValidationResult {
            definition_digest: digest_value(definition).map_err(ManagementError::from)?,
            compiler: "test-compiler.v1".to_owned(),
            diagnostics: Vec::<Diagnostic>::new(),
        })
    }

    fn inspect(
        &self,
        _definition: &Value,
    ) -> Result<sts2_harness::management::InspectionResult, ManagementError> {
        Err(ManagementError::unavailable(
            "unused",
            "inspection is not used by this test",
        ))
    }

    fn diff(
        &self,
        _old_definition: &Value,
        _new_definition: &Value,
    ) -> Result<sts2_harness::management::DiffResult, ManagementError> {
        Err(ManagementError::unavailable(
            "unused",
            "diff is not used by this test",
        ))
    }
}

fn actor() -> Result<AuthContext, sts2_harness::management::AuthError> {
    AuthContext::new("operator", ["workflow:*".to_owned()])
}

fn request(definition: Value) -> RunRequest {
    RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: "request-target-admission".to_owned(),
        definition: Some(definition),
        artifact_id: None,
        instance_id: "instance-1".to_owned(),
        profile: "synthetic".to_owned(),
        admission: None,
    }
}

fn target_descriptor() -> TargetDescriptor {
    target_descriptor_with_operations(vec![
        "workflow:control".to_owned(),
        "workflow:live".to_owned(),
    ])
}

fn target_descriptor_with_operations(supported_operations: Vec<String>) -> TargetDescriptor {
    TargetDescriptor {
        instance_id: "instance-1".to_owned(),
        execution_profiles: vec!["live.workflow.v1".to_owned()],
        execution_mode: ExecutionMode::Live,
        compatibility_revision: "live.compatibility.v1".to_owned(),
        capability_revision: "live.capabilities.v1".to_owned(),
        availability: TargetAvailability::Available,
        supported_operations,
        capabilities: vec!["workflow.live".to_owned()],
        game_profiles: vec!["sts2-live-v1".to_owned()],
        save_profiles: Vec::new(),
        inference_profiles: Vec::new(),
    }
}

struct ScopedCapabilityDouble {
    supported_operations: Vec<String>,
}

impl CapabilityPort for ScopedCapabilityDouble {
    fn capabilities(&self) -> Result<Value, ManagementError> {
        Ok(json!({
            "schema_version": "ascension.capabilities/v1",
            "capabilities": ["workflow.live"]
        }))
    }

    fn target_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        if actor.subject != "operator" {
            return Err(ManagementError::forbidden(
                "target_scope_denied",
                "target discovery is not available to this actor",
            ));
        }
        let mut descriptor = target_descriptor();
        descriptor.supported_operations = self.supported_operations.clone();
        Ok(TargetCatalogResponse {
            schema_version: "ascension.workflow-targets/v1".to_owned(),
            catalog_revision: "live.catalog.v1".to_owned(),
            targets: vec![descriptor],
        })
    }
}

struct SubmissionDouble {
    submissions: AtomicUsize,
}

impl SubmissionDouble {
    fn new() -> Self {
        Self {
            submissions: AtomicUsize::new(0),
        }
    }
}

impl WorkflowExecutionPort for SubmissionDouble {
    fn submit(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        definition_digest: &str,
    ) -> Result<RunAdmission, ManagementError> {
        self.submissions.fetch_add(1, Ordering::SeqCst);
        let snapshot = RunSnapshot {
            schema_version: "ascension.workflow-run/v1".to_owned(),
            workflow_run_id: "run-http-target".to_owned(),
            definition_digest: definition_digest.to_owned(),
            run_revision: 1,
            status: WorkflowRunStatus::Created,
            game_outcome: GameOutcome::NotTerminal,
            cursor: sts2_harness::management::Cursor {
                graph_id: "graph".to_owned(),
                node_id: "node".to_owned(),
                node_execution_id: "node-exec".to_owned(),
            },
            pending_operation: None,
            budget: Budget::default(),
            cleanup: CleanupState::NotStarted,
            admission: None,
            execution_mode: None,
        };
        Ok(RunAdmission {
            initial_events: vec![RunEvent {
                schema_version: EVENT_SCHEMA_VERSION.to_owned(),
                workflow_run_id: snapshot.workflow_run_id.clone(),
                sequence: 1,
                run_revision: 1,
                event_type: EventType::RunStarted,
                definition_digest: definition_digest.to_owned(),
                node_execution_id: "management".to_owned(),
                payload: EventPayload {
                    operation_id: None,
                    classification: Some(EventClassification::Accepted),
                    reason_code: "submitted".to_owned(),
                },
                integrity_digest: None,
            }],
            snapshot,
        })
    }

    fn apply_command(
        &self,
        _context: CommandContext,
    ) -> Result<CommandApplication, ManagementError> {
        Err(ManagementError::unavailable(
            "unused",
            "commands are not used by this test",
        ))
    }
}

#[test]
fn live_requires_binding_and_stale_binding_is_rejected_before_execution()
-> Result<(), Box<dyn std::error::Error>> {
    let service = ManagementService::in_memory().with_definition_port(Arc::new(DefinitionDouble));
    let actor = actor()?;

    let mut missing = request(json!({"workflow": "fixture"}));
    missing.profile = "live.workflow.v1".to_owned();
    let missing_error = service
        .submit_run(&actor, missing)
        .err()
        .ok_or("live submission without admission unexpectedly succeeded")?;
    assert_eq!(missing_error.code, "target_admission_required");

    let definition: Value = serde_json::from_slice(include_bytes!(
        "../../../conformance/workflow-v1/valid-strict.json"
    ))?;
    let mut stale = request(definition.clone());
    stale.admission = Some(TargetAdmissionBinding {
        schema_version: "ascension.workflow-admission/v1".to_owned(),
        request_id: stale.request_id.clone(),
        workflow_definition_digest: digest_value(&definition)?,
        target: RunTargetConfiguration {
            instance_id: stale.instance_id.clone(),
            execution_profile: stale.profile.clone(),
            execution_mode: ExecutionMode::Synthetic,
            workflow_revision: "9.9.9".to_owned(),
            compatibility_revision: "synthetic.compatibility.v1".to_owned(),
            capability_revision: "synthetic.capabilities.v1".to_owned(),
            game_profile: "synthetic-sts2-v1".to_owned(),
            save_profile: None,
            inference_profile: None,
            context_capability: None,
            provider_capability: None,
        },
        descriptor_digest: digest_value(&json!({"descriptor": "fixture"}))?,
        catalog_revision: "synthetic.catalog.v1".to_owned(),
    });
    let stale_error = service
        .submit_run(&actor, stale)
        .err()
        .ok_or("stale admission unexpectedly succeeded")?;
    assert_eq!(stale_error.code, "target_admission_stale");
    Ok(())
}

#[test]
fn live_target_admission_requires_live_operation_support() -> Result<(), Box<dyn std::error::Error>>
{
    let submissions = Arc::new(SubmissionDouble::new());
    let service = ManagementService::in_memory()
        .with_capability_port(Arc::new(ScopedCapabilityDouble {
            supported_operations: vec!["workflow:read".to_owned()],
        }))
        .with_definition_port(Arc::new(DefinitionDouble))
        .with_context_owner_port(Arc::new(FakeContextOwner))
        .with_execution_port(submissions.clone());
    let actor = actor()?;
    let target = RunTargetConfiguration {
        instance_id: "instance-1".to_owned(),
        execution_profile: "live.workflow.v1".to_owned(),
        execution_mode: ExecutionMode::Live,
        workflow_revision: "0.1.0".to_owned(),
        compatibility_revision: "live.compatibility.v1".to_owned(),
        capability_revision: "live.capabilities.v1".to_owned(),
        game_profile: "sts2-live-v1".to_owned(),
        save_profile: None,
        inference_profile: None,
        context_capability: None,
        provider_capability: None,
    };
    let definition = live_workflow_definition::definition()?;
    let workflow_definition_digest = digest_value(&definition)?;
    let preflight_error = service
        .preflight_target(
            &actor,
            TargetAdmissionRequest {
                schema_version: "ascension.workflow-admission/v1".to_owned(),
                request_id: "request-operation-support".to_owned(),
                workflow_definition_digest: workflow_definition_digest.clone(),
                target: target.clone(),
            },
        )
        .err()
        .ok_or_else(|| {
            std::io::Error::other(
                "live preflight without workflow:live support unexpectedly succeeded",
            )
        })?;
    assert_eq!(preflight_error.code, "target_operation_unavailable");

    let descriptor = target_descriptor_with_operations(vec!["workflow:read".to_owned()]);
    let submit_error = service
        .submit_run(
            &actor,
            RunRequest {
                schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
                request_id: "request-operation-support".to_owned(),
                definition: Some(definition),
                artifact_id: None,
                instance_id: "instance-1".to_owned(),
                profile: "live.workflow.v1".to_owned(),
                admission: Some(TargetAdmissionBinding {
                    schema_version: "ascension.workflow-admission/v1".to_owned(),
                    request_id: "request-operation-support".to_owned(),
                    workflow_definition_digest,
                    target,
                    descriptor_digest: descriptor.digest()?,
                    catalog_revision: "live.catalog.v1".to_owned(),
                }),
            },
        )
        .err()
        .ok_or_else(|| {
            std::io::Error::other(
                "live submission without workflow:live support unexpectedly succeeded",
            )
        })?;
    assert_eq!(submit_error.code, "target_operation_unavailable");
    assert_eq!(submissions.submissions.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn authenticated_scoped_catalog_preflight_and_submission_are_actor_bound()
-> Result<(), Box<dyn std::error::Error>> {
    let submissions = Arc::new(SubmissionDouble::new());
    let execution: Arc<dyn WorkflowExecutionPort> = submissions.clone();
    let service = Arc::new(
        ManagementService::in_memory()
            .with_capability_port(Arc::new(ScopedCapabilityDouble {
                supported_operations: vec![
                    "workflow:control".to_owned(),
                    "workflow:live".to_owned(),
                ],
            }))
            .with_definition_port(Arc::new(DefinitionDouble))
            .with_context_owner_port(Arc::new(FakeContextOwner))
            .with_execution_port(execution),
    );
    let operator = AuthContext::new("operator", ["workflow:*".to_owned()])?;
    let reader = AuthContext::new("reader", ["workflow:read".to_owned()])?;
    let authenticator = StaticAuthenticator::new()
        .with_credential("operator-token", operator.clone())?
        .with_credential("reader-token", reader)?;
    let config = ServerConfig::new(
        "127.0.0.1:0".parse::<SocketAddr>()?,
        Arc::new(authenticator),
    )?;
    let server = ManagementServer::start(config, service)?;
    let address = server.address();
    let operator_client = ManagementClient::new(address, "operator-token")?;
    let reader_client = ManagementClient::new(address, "reader-token")?;

    let catalog_response = operator_client.request_json("GET", "/v1/workflow-targets", None)?;
    assert_eq!(catalog_response.status, 200);
    let catalog: TargetCatalogResponse = decode_strict(&catalog_response.body)?;
    catalog.validate()?;
    assert_eq!(catalog.targets[0].instance_id, "instance-1");

    let hidden = reader_client.request_json("GET", "/v1/workflow-targets", None)?;
    assert_eq!(hidden.status, 403);
    let unauthorized = ManagementClient::new(address, "unknown-token")?.request_json(
        "GET",
        "/v1/workflow-targets",
        None,
    )?;
    assert_eq!(unauthorized.status, 401);

    let request_id = "request-http-target";
    let definition = live_workflow_definition::definition()?;
    let workflow_definition_digest = digest_value(&definition)?;
    let target = RunTargetConfiguration {
        instance_id: "instance-1".to_owned(),
        execution_profile: "live.workflow.v1".to_owned(),
        execution_mode: ExecutionMode::Live,
        workflow_revision: "0.1.0".to_owned(),
        compatibility_revision: "live.compatibility.v1".to_owned(),
        capability_revision: "live.capabilities.v1".to_owned(),
        game_profile: "sts2-live-v1".to_owned(),
        save_profile: None,
        inference_profile: None,
        context_capability: None,
        provider_capability: None,
    };
    let preflight_body = serde_json::to_vec(&TargetAdmissionRequest {
        schema_version: "ascension.workflow-admission/v1".to_owned(),
        request_id: request_id.to_owned(),
        workflow_definition_digest: workflow_definition_digest.clone(),
        target,
    })?;
    let preflight_response = operator_client.request_json(
        "POST",
        "/v1/workflow-targets/preflight",
        Some(&preflight_body),
    )?;
    assert_eq!(preflight_response.status, 200);
    let preflight: TargetPreflightResponse = decode_strict(&preflight_response.body)?;
    preflight.validate()?;
    assert_eq!(preflight.admission.request_id, request_id);
    assert_eq!(preflight.admission.target.instance_id, "instance-1");
    assert_eq!(submissions.submissions.load(Ordering::SeqCst), 0);

    let run_body = serde_json::to_vec(&RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: request_id.to_owned(),
        definition: Some(definition),
        artifact_id: None,
        instance_id: "instance-1".to_owned(),
        profile: "live.workflow.v1".to_owned(),
        admission: Some(preflight.admission.clone()),
    })?;
    let run_response =
        operator_client.request_json("POST", "/v1/workflow-runs", Some(&run_body))?;
    assert_eq!(run_response.status, 200);
    assert_eq!(submissions.submissions.load(Ordering::SeqCst), 1);
    server.shutdown()?;
    Ok(())
}
