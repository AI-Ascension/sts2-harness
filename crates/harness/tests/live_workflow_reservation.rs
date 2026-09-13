// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use serde_json::json;
use sts2_harness::management::{
    AuthContext, LiveWorkflowExecutionPort, LiveWorkflowOptions, LiveWorkflowSession,
    LiveWorkflowSessionFactory, ManagementError, MemoryWorkflowStore, RUN_SCHEMA_VERSION,
    RunRequest, RunSnapshot, TargetCatalogResponse, WorkflowExecutionPort, WorkflowStore,
    decode_strict, digest_value, live_store,
};

#[path = "support/live_workflow.rs"]
mod support;

use support::*;

struct ReservationObservingFactory {
    inner: support::FakeFactory,
    store: Arc<dyn WorkflowStore>,
    opens: Arc<AtomicUsize>,
}

impl ReservationObservingFactory {
    fn new(store: Arc<dyn WorkflowStore>) -> (Self, Arc<AtomicUsize>) {
        let opens = Arc::new(AtomicUsize::new(0));
        (
            Self {
                inner: support::FakeFactory::new(false),
                store,
                opens: Arc::clone(&opens),
            },
            opens,
        )
    }
}

impl LiveWorkflowSessionFactory for ReservationObservingFactory {
    fn capabilities(&self) -> serde_json::Value {
        self.inner.capabilities()
    }

    fn target_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        self.inner.target_catalog(actor)
    }

    fn open(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &sts2_harness::workflow::WorkflowDefinition,
        definition_digest: &str,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        let run_identity = json!({
            "request_id": request.request_id,
            "instance_id": request.instance_id,
            "definition_digest": definition_digest,
        });
        let run_digest = digest_value(&run_identity).map_err(ManagementError::from)?;
        let run_id = format!("run.live.{}", &run_digest[..32]);
        match self.store.get_run(&run_id).map_err(ManagementError::from)? {
            Some(snapshot) if snapshot.admission == request.admission => {}
            _ => {
                return Err(ManagementError::conflict(
                    "reservation_not_persisted",
                    "live factory opened before the exact admission was persisted",
                ));
            }
        }
        self.inner
            .open(request, actor, definition, definition_digest)
    }
}

#[test]
fn reservation_failure_precedes_factory_open_and_launch() {
    let factory = Arc::new(support::FakeFactory::new(false));
    let port = LiveWorkflowExecutionPort::new(
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("port");
    let request = request("request-reservation-rejected", definition(false));
    let binding = request.admission.as_ref();
    let definition_digest =
        digest_value(request.definition.as_ref().expect("definition")).expect("definition digest");
    let error = port
        .submit_admitted_with_reservation(&request, &actor(), &definition_digest, binding, &|_| {
            Err(ManagementError::store(
                "reservation_rejected",
                "fixture store rejected the pre-effect reservation",
            ))
        })
        .expect_err("reservation failure");
    assert_eq!(error.code, "reservation_rejected");
    assert!(
        factory.entries().is_empty(),
        "factory.open and session.launch must not run after reservation failure"
    );
}

#[test]
fn exact_admission_binding_is_durable_before_factory_open() {
    let store = Arc::new(MemoryWorkflowStore::new());
    let (factory, opens) =
        ReservationObservingFactory::new(Arc::clone(&store) as Arc<dyn WorkflowStore>);
    let factory = Arc::new(factory);
    let service = live_store(
        Arc::clone(&store) as Arc<dyn WorkflowStore>,
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let request = request("request-reservation-order", definition(false));
    let expected = request.admission.clone();

    let submitted = service
        .submit_run(&actor(), request.clone())
        .expect("submit");
    let persisted = store
        .get_run(&submitted.workflow_run_id)
        .expect("lookup")
        .expect("durable run");
    assert_eq!(persisted.admission, expected);
    assert_eq!(opens.load(Ordering::SeqCst), 1);
    assert_eq!(factory.inner.entries(), ["launch"]);

    let retry = service
        .submit_run(&actor(), request)
        .expect("idempotent retry");
    assert_eq!(retry.workflow_run_id, submitted.workflow_run_id);
    assert_eq!(opens.load(Ordering::SeqCst), 1);
}

#[test]
fn legacy_snapshot_without_admission_remains_decodable() -> Result<(), Box<dyn std::error::Error>> {
    let legacy = json!({
        "schema_version": RUN_SCHEMA_VERSION,
        "workflow_run_id": "run-legacy",
        "definition_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "run_revision": 1,
        "status": "created",
        "game_outcome": "not_terminal",
        "cursor": {
            "graph_id": "graph",
            "node_id": "node",
            "node_execution_id": "exec"
        },
        "pending_operation": null,
        "budget": {
            "provider_calls_consumed": 0,
            "provider_calls_reserved": 0,
            "node_steps_consumed": 0,
            "replans_consumed": 0
        },
        "cleanup": "not_started"
    });
    let snapshot: RunSnapshot = decode_strict(&serde_json::to_vec(&legacy)?)?;
    assert_eq!(snapshot.schema_version, RUN_SCHEMA_VERSION);
    assert!(snapshot.admission.is_none());
    Ok(())
}
