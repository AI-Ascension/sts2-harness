// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::sync::{Arc, Mutex};

use sts2_harness::management::{
    AuthContext, CommandKind, ContextBindingCatalog, ContextBindingRequest, ContextOwnerBinding,
    ContextOwnerPort, LiveWorkflowOptions, LiveWorkflowSessionFactory, ManagementError,
    MemoryWorkflowStore,
};

#[allow(clippy::duplicate_mod)]
#[path = "support/live_workflow_context_owner.rs"]
mod owner_double;
#[path = "support/live_workflow.rs"]
mod support;

use owner_double::FakeContextOwner;
use support::*;

/// Records every owner binding request and can optionally return a tampered or
/// grant-escalated binding so the dispatch validator is exercised.
struct RecordingOwner {
    requests: Mutex<Vec<ContextBindingRequest>>,
    tamper_execution_id: Option<String>,
    escalate_control: bool,
}

impl RecordingOwner {
    fn new() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            tamper_execution_id: None,
            escalate_control: false,
        }
    }

    fn with_tampered_execution_id(id: &str) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            tamper_execution_id: Some(id.to_owned()),
            escalate_control: false,
        }
    }

    fn with_escalated_control() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            tamper_execution_id: None,
            escalate_control: true,
        }
    }

    fn recorded(&self) -> Vec<ContextBindingRequest> {
        self.requests.lock().expect("request lock").clone()
    }
}

impl ContextOwnerPort for RecordingOwner {
    fn catalog(&self, actor: &AuthContext) -> Result<ContextBindingCatalog, ManagementError> {
        <FakeContextOwner as ContextOwnerPort>::catalog(&FakeContextOwner, actor)
    }

    fn bind(
        &self,
        actor: &AuthContext,
        request: &ContextBindingRequest,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        let mut binding =
            <FakeContextOwner as ContextOwnerPort>::bind(&FakeContextOwner, actor, request)?;
        self.requests
            .lock()
            .map_err(|_| {
                ManagementError::invalid("test_owner_lock", "recording owner lock is poisoned")
            })?
            .push(request.clone());
        if let Some(id) = &self.tamper_execution_id {
            binding.node_execution_id = id.clone();
        }
        if self.escalate_control {
            binding.grants.control = true;
        }
        Ok(binding)
    }

    fn is_available(&self) -> bool {
        true
    }
}

fn live_service_with_owner(
    owner: Arc<RecordingOwner>,
) -> Result<sts2_harness::management::ManagementService, ManagementError> {
    let factory = Arc::new(FakeFactory::new(false));
    live_service(
        Arc::new(MemoryWorkflowStore::new()),
        factory as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .map(|service| service.with_context_owner_port(owner as Arc<dyn ContextOwnerPort>))
}

fn observe_then_decide(
    service: &sts2_harness::management::ManagementService,
    run_id: &str,
) -> Result<(), ManagementError> {
    let actor = actor();
    service.command(&actor, command(run_id, "step-1", 1, CommandKind::Step))?;
    service
        .command(&actor, command(run_id, "step-2", 2, CommandKind::Step))
        .map(|_| ())
}

#[test]
fn context_owner_binds_context_node_at_runtime_allocated_invocation()
-> Result<(), Box<dyn std::error::Error>> {
    let owner = Arc::new(RecordingOwner::new());
    let service = live_service_with_owner(Arc::clone(&owner))?;
    let actor = actor();
    let submitted = service.submit_run(&actor, request("request-dispatch-1", definition(false)))?;
    let run_id = submitted.workflow_run_id;

    // The entry `observe` node is not context-bound, so admission and the first
    // step must not ask the owner for a binding.
    service.command(&actor, command(&run_id, "step-1", 1, CommandKind::Step))?;
    assert!(
        owner.recorded().is_empty(),
        "a non-context entry node must not be bound"
    );

    // `decide` is context-bound and the runtime allocates `live.node.2` to it.
    service.command(&actor, command(&run_id, "step-2", 2, CommandKind::Step))?;
    let requests = owner.recorded();
    assert_eq!(requests.len(), 1, "decide must be bound exactly once");
    let request = &requests[0];
    assert_eq!(request.node_id, "decide");
    assert_eq!(request.node_kind, "decide");
    assert_eq!(request.context_ref, "context.live.v1");
    assert_eq!(
        request.node_execution_id, "live.node.2",
        "binding must use the runtime-allocated invocation, not a hard-coded first step"
    );
    assert_eq!(request.workflow_run_id, run_id);
    Ok(())
}

#[test]
fn context_owner_rejects_a_binding_for_a_different_invocation()
-> Result<(), Box<dyn std::error::Error>> {
    let owner = Arc::new(RecordingOwner::with_tampered_execution_id("live.node.1"));
    let service = live_service_with_owner(Arc::clone(&owner))?;
    let actor = actor();
    let submitted = service.submit_run(&actor, request("request-dispatch-2", definition(false)))?;
    let run_id = submitted.workflow_run_id;

    let error = observe_then_decide(&service, &run_id)
        .expect_err("a binding for a different invocation must fail closed");
    assert_eq!(error.code, "context_owner_binding_execution_mismatch");
    Ok(())
}

#[test]
fn context_owner_rejects_a_dispatch_binding_that_escalates_grants()
-> Result<(), Box<dyn std::error::Error>> {
    let owner = Arc::new(RecordingOwner::with_escalated_control());
    let service = live_service_with_owner(Arc::clone(&owner))?;
    let actor = actor();
    let submitted = service.submit_run(&actor, request("request-dispatch-3", definition(false)))?;
    let run_id = submitted.workflow_run_id;

    let error = observe_then_decide(&service, &run_id)
        .expect_err("a dispatch binding that escalates grants must fail closed");
    assert_eq!(error.code, "context_owner_binding_grant_escalation");
    Ok(())
}
