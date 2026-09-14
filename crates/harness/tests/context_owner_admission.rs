// SPDX-License-Identifier: MIT

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use sts2_harness::management::{
    AuthContext, Budget, CapabilityPort, CleanupState, CommandApplication, CommandContext,
    ContextBindingCatalog, ContextBindingRequest, ContextBindingState, ContextOwnerBinding,
    ContextOwnerPort, DefinitionPort, Diagnostic, EVENT_SCHEMA_VERSION, EventClassification,
    EventPayload, EventType, ExecutionMode, GameOutcome, MANAGEMENT_SCHEMA_VERSION,
    ManagementError, ManagementService, RunAdmission, RunEvent, RunRequest, RunSnapshot,
    RunTargetConfiguration, TargetAdmissionBinding, TargetAvailability, TargetCatalogResponse,
    TargetDescriptor, ValidationResult, WorkflowExecutionPort, WorkflowRunStatus, digest_value,
};

#[path = "support/live_workflow_context_owner.rs"]
mod context_owner_double;

use context_owner_double::FakeContextOwner;

fn actor() -> Result<AuthContext, sts2_harness::management::AuthError> {
    AuthContext::new("operator", ["workflow:*".to_owned()])
}

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
        Err(ManagementError::unavailable("unused", "unused"))
    }

    fn diff(
        &self,
        _old_definition: &Value,
        _new_definition: &Value,
    ) -> Result<sts2_harness::management::DiffResult, ManagementError> {
        Err(ManagementError::unavailable("unused", "unused"))
    }
}

struct OrderedCapabilityDouble {
    target_catalog_calls: Arc<AtomicUsize>,
}

impl CapabilityPort for OrderedCapabilityDouble {
    fn capabilities(&self) -> Result<Value, ManagementError> {
        Ok(json!({
            "schema_version": "ascension.capabilities/v1",
            "capabilities": ["workflow.live"]
        }))
    }

    fn target_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        self.target_catalog_calls.fetch_add(1, Ordering::SeqCst);
        Ok(TargetCatalogResponse {
            schema_version: "ascension.workflow-targets/v1".to_owned(),
            catalog_revision: "live.catalog.v1".to_owned(),
            targets: vec![target_descriptor()],
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
        Ok(RunAdmission {
            initial_events: vec![RunEvent {
                schema_version: EVENT_SCHEMA_VERSION.to_owned(),
                workflow_run_id: "run-owner-gate".to_owned(),
                sequence: 1,
                run_revision: 1,
                event_type: EventType::RunStarted,
                definition_digest: definition_digest.to_owned(),
                node_execution_id: "node-exec".to_owned(),
                payload: EventPayload {
                    operation_id: None,
                    classification: Some(EventClassification::Accepted),
                    reason_code: "test".to_owned(),
                },
                integrity_digest: None,
            }],
            snapshot: RunSnapshot {
                schema_version: "ascension.workflow-run/v1".to_owned(),
                workflow_run_id: "run-owner-gate".to_owned(),
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
            },
        })
    }

    fn apply_command(
        &self,
        _context: CommandContext,
    ) -> Result<CommandApplication, ManagementError> {
        Err(ManagementError::unavailable("unused", "unused"))
    }
}

fn target_descriptor() -> TargetDescriptor {
    TargetDescriptor {
        instance_id: "instance-1".to_owned(),
        execution_profiles: vec!["live.workflow.v1".to_owned()],
        execution_mode: ExecutionMode::Live,
        compatibility_revision: "live.compatibility.v1".to_owned(),
        capability_revision: "live.capabilities.v1".to_owned(),
        availability: TargetAvailability::Available,
        supported_operations: vec!["workflow:live".to_owned()],
        capabilities: vec!["workflow.live".to_owned()],
        game_profiles: vec!["sts2-live-v1".to_owned()],
        save_profiles: Vec::new(),
        inference_profiles: Vec::new(),
    }
}

fn live_definition() -> Result<Value, Box<dyn std::error::Error>> {
    let mut value: Value = serde_json::from_slice(include_bytes!(
        "../../../conformance/workflow-v1/valid-strict.json"
    ))?;
    value["annotations"]["synthetic"] = json!(false);
    value["game_profile"] = json!("sts2-live-v1");
    value["policy_ref"] = json!("policy.live.v1");
    value["graphs"][0]["nodes"][0]["config"]["projection_ref"] = json!("fair-play.live.v1");
    value["graphs"][0]["nodes"][1]["config"]["decision_profile_ref"] = json!("decision.live.v1");
    value["graphs"][0]["nodes"][1]["config"]["context_ref"] = json!("context.live.v1");
    value["capabilities"]["required"][0] = json!("observe.fair-play.live.v1");
    Ok(value)
}

fn admitted_live_request(
    definition: Value,
    request_id: &str,
) -> Result<RunRequest, Box<dyn std::error::Error>> {
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
    let descriptor = target_descriptor();
    Ok(RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: request_id.to_owned(),
        definition: Some(definition),
        artifact_id: None,
        instance_id: "instance-1".to_owned(),
        profile: "live.workflow.v1".to_owned(),
        admission: Some(TargetAdmissionBinding {
            schema_version: "ascension.workflow-admission/v1".to_owned(),
            request_id: request_id.to_owned(),
            workflow_definition_digest,
            target,
            descriptor_digest: descriptor.digest()?,
            catalog_revision: "live.catalog.v1".to_owned(),
        }),
    })
}

#[derive(Clone, Copy)]
enum OwnerGate {
    Missing,
    Denied,
    Unavailable,
    Ambiguous,
    BindDenied,
    BindGrantEscalated,
    BindContinuityEscalated,
}

struct CatalogGateOwner {
    gate: OwnerGate,
    bind_calls: Arc<AtomicUsize>,
}

impl ContextOwnerPort for CatalogGateOwner {
    fn catalog(&self, actor: &AuthContext) -> Result<ContextBindingCatalog, ManagementError> {
        if matches!(self.gate, OwnerGate::Unavailable) {
            return Err(ManagementError::unavailable(
                "context_owner_catalog_unavailable",
                "catalog is unavailable for this actor",
            ));
        }
        let mut catalog =
            <FakeContextOwner as ContextOwnerPort>::catalog(&FakeContextOwner, actor)?;
        match self.gate {
            OwnerGate::Missing => catalog.descriptors.clear(),
            OwnerGate::Denied => {
                let mut descriptor = catalog.descriptors.remove(0);
                descriptor.state = ContextBindingState::Denied;
                catalog.descriptors.push(descriptor.seal()?);
            }
            OwnerGate::Ambiguous => {
                let mut duplicate = catalog.descriptors[0].clone();
                duplicate.binding_id = "fake.binding.2".to_owned();
                duplicate.version = 2;
                duplicate.digest.clear();
                catalog.descriptors.push(duplicate.seal()?);
            }
            OwnerGate::Unavailable
            | OwnerGate::BindDenied
            | OwnerGate::BindGrantEscalated
            | OwnerGate::BindContinuityEscalated => {}
        }
        let owner_id = catalog.owner_id.clone();
        let owner_version = catalog.owner_version.clone();
        let bytes = serde_json::to_vec(&(&owner_id, &owner_version, &catalog.descriptors))
            .map_err(|error| {
                ManagementError::invalid("context_catalog_encode", error.to_string())
            })?;
        catalog.catalog_digest = sts2_harness::sha256_hex(bytes);
        catalog.validate()?;
        Ok(catalog)
    }

    fn bind(
        &self,
        actor: &AuthContext,
        request: &ContextBindingRequest,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        self.bind_calls.fetch_add(1, Ordering::SeqCst);
        if matches!(
            self.gate,
            OwnerGate::BindGrantEscalated | OwnerGate::BindContinuityEscalated
        ) {
            let mut binding =
                <FakeContextOwner as ContextOwnerPort>::bind(&FakeContextOwner, actor, request)?;
            if matches!(self.gate, OwnerGate::BindGrantEscalated) {
                binding.grants.control = true;
            } else {
                binding.continuity.provider_session_continuity = true;
            }
            return Ok(binding);
        }
        Err(ManagementError::capability(
            "context_binding_denied",
            "owner denied this invocation binding",
        ))
    }

    fn is_available(&self) -> bool {
        true
    }
}

#[test]
fn context_owner_catalog_gates_live_admission_before_effects()
-> Result<(), Box<dyn std::error::Error>> {
    let expected = [
        (OwnerGate::Missing, "context_binding_unsupported"),
        (OwnerGate::Denied, "context_binding_unsupported"),
        (OwnerGate::Unavailable, "context_owner_catalog_unavailable"),
        (OwnerGate::Ambiguous, "context_binding_ambiguous"),
    ];
    for (index, (gate, expected_code)) in expected.into_iter().enumerate() {
        let submissions = Arc::new(SubmissionDouble::new());
        let target_catalog_calls = Arc::new(AtomicUsize::new(0));
        let bind_calls = Arc::new(AtomicUsize::new(0));
        let service = ManagementService::in_memory()
            .with_definition_port(Arc::new(DefinitionDouble))
            .with_capability_port(Arc::new(OrderedCapabilityDouble {
                target_catalog_calls: Arc::clone(&target_catalog_calls),
            }))
            .with_context_owner_port(Arc::new(CatalogGateOwner {
                gate,
                bind_calls: Arc::clone(&bind_calls),
            }))
            .with_execution_port(submissions.clone());
        let request =
            admitted_live_request(live_definition()?, &format!("request-owner-gate-{index}"))?;
        let error = service
            .submit_run(&actor()?, request)
            .err()
            .ok_or("owner gate accepted the live submission")?;
        assert_eq!(error.code, expected_code);
        assert_eq!(target_catalog_calls.load(Ordering::SeqCst), 0);
        assert_eq!(submissions.submissions.load(Ordering::SeqCst), 0);
        // Admission performs a bounded catalog/support check only; it must not
        // fabricate a per-invocation binding before the runtime allocates one.
        assert_eq!(bind_calls.load(Ordering::SeqCst), 0);
    }
    Ok(())
}

#[test]
fn context_owner_bind_failures_are_deferred_to_dispatch() -> Result<(), Box<dyn std::error::Error>>
{
    // Owner `bind` responses (denied or escalating) are resolved at dispatch,
    // where the runtime-allocated invocation identity is known. Admission
    // therefore still adopts the run and reaches the target/execution effects;
    // the dispatch path is exercised by `context_owner_dispatch`.
    for (index, gate) in [
        OwnerGate::BindDenied,
        OwnerGate::BindGrantEscalated,
        OwnerGate::BindContinuityEscalated,
    ]
    .into_iter()
    .enumerate()
    {
        let submissions = Arc::new(SubmissionDouble::new());
        let target_catalog_calls = Arc::new(AtomicUsize::new(0));
        let bind_calls = Arc::new(AtomicUsize::new(0));
        let service = ManagementService::in_memory()
            .with_definition_port(Arc::new(DefinitionDouble))
            .with_capability_port(Arc::new(OrderedCapabilityDouble {
                target_catalog_calls: Arc::clone(&target_catalog_calls),
            }))
            .with_context_owner_port(Arc::new(CatalogGateOwner {
                gate,
                bind_calls: Arc::clone(&bind_calls),
            }))
            .with_execution_port(submissions.clone());
        let request = admitted_live_request(
            live_definition()?,
            &format!("request-owner-dispatch-{index}"),
        )?;
        service.submit_run(&actor()?, request)?;
        assert_eq!(bind_calls.load(Ordering::SeqCst), 0);
        assert_eq!(target_catalog_calls.load(Ordering::SeqCst), 1);
        assert_eq!(submissions.submissions.load(Ordering::SeqCst), 1);
    }
    Ok(())
}
