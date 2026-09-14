// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::sync::{Arc, Mutex};

use sts2_harness::management::{
    AuthContext, CommandContext, CommandKind, ContextBindingCatalog, ContextBindingRequest,
    ContextOwnerBinding, ContextOwnerPort, LiveWorkflowExecutionPort, LiveWorkflowOptions,
    LiveWorkflowSessionFactory, ManagementError, MemoryWorkflowStore, WorkflowExecutionPort,
    WorkflowStore, live_store,
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

#[test]
fn context_owner_rejects_a_binding_that_does_not_match_the_persisted_cursor()
-> Result<(), Box<dyn std::error::Error>> {
    // A response that is internally consistent with the dispatch request but is
    // not attached to the persisted run cursor must fail closed. This is the
    // guard that the admission-time fabricated identity could never satisfy.
    let factory = Arc::new(FakeFactory::new(false));
    let store = Arc::new(MemoryWorkflowStore::new());
    let owner = Arc::new(RecordingOwner::new());
    let execution = Arc::new(LiveWorkflowExecutionPort::new(
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )?);
    let service = live_store(
        Arc::clone(&store) as Arc<dyn WorkflowStore>,
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )?
    .with_execution_port(Arc::clone(&execution) as Arc<dyn WorkflowExecutionPort>)
    .with_context_owner_port(Arc::clone(&owner) as Arc<dyn ContextOwnerPort>);

    let actor = actor();
    let submitted = service.submit_run(
        &actor,
        request("request-dispatch-cursor", definition(false)),
    )?;
    let run_id = submitted.workflow_run_id;
    service.command(&actor, command(&run_id, "step-1", 1, CommandKind::Step))?;

    let mut snapshot = store
        .get_run(&run_id)
        .expect("run lookup")
        .expect("persisted run");
    snapshot.cursor.node_execution_id = "live.node.99".to_owned();

    let error = execution
        .apply_command(CommandContext {
            request: command(&run_id, "step-2", 2, CommandKind::Step),
            snapshot,
            actor,
            context_owner: Arc::clone(&owner) as Arc<dyn ContextOwnerPort>,
        })
        .expect_err("a binding that does not match the persisted cursor must fail closed");
    assert_eq!(error.code, "context_owner_cursor_mismatch");
    assert!(
        !factory.entries().iter().any(|entry| entry == "decide"),
        "a rejected binding must not execute the context node"
    );
    Ok(())
}

#[test]
fn dispatch_owner_is_isolated_between_services_sharing_one_execution_adapter()
-> Result<(), Box<dyn std::error::Error>> {
    // The owner travels with the command, so composing a second service over the
    // same execution adapter must not replace the first service's owner. Both
    // builder orders are exercised.
    let factory = Arc::new(FakeFactory::new(false));
    let execution = Arc::new(LiveWorkflowExecutionPort::new(
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )?);
    let owner_a = Arc::new(RecordingOwner::new());
    let owner_b = Arc::new(RecordingOwner::new());
    let service_a = live_store(
        Arc::new(MemoryWorkflowStore::new()),
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )?
    .with_execution_port(Arc::clone(&execution) as Arc<dyn WorkflowExecutionPort>)
    .with_context_owner_port(Arc::clone(&owner_a) as Arc<dyn ContextOwnerPort>);
    let service_b = live_store(
        Arc::new(MemoryWorkflowStore::new()),
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )?
    .with_context_owner_port(Arc::clone(&owner_b) as Arc<dyn ContextOwnerPort>)
    .with_execution_port(Arc::clone(&execution) as Arc<dyn WorkflowExecutionPort>);

    let actor = actor();
    let run_a = service_a
        .submit_run(&actor, request("request-shared-a", definition(false)))?
        .workflow_run_id;
    let run_b = service_b
        .submit_run(&actor, request("request-shared-b", definition(false)))?
        .workflow_run_id;

    observe_then_decide(&service_a, &run_a)?;
    observe_then_decide(&service_b, &run_b)?;

    let requests_a = owner_a.recorded();
    let requests_b = owner_b.recorded();
    assert_eq!(requests_a.len(), 1, "service A must bind through owner A");
    assert_eq!(requests_b.len(), 1, "service B must bind through owner B");
    assert!(
        requests_a
            .iter()
            .all(|request| request.workflow_run_id == run_a),
        "owner A must only be used for service A's run"
    );
    assert!(
        requests_b
            .iter()
            .all(|request| request.workflow_run_id == run_b),
        "owner B must only be used for service B's run"
    );
    Ok(())
}
