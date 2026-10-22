// SPDX-License-Identifier: MIT

//! Synthetic live execution boundary: the durable run exists before the open port stalls.

#![allow(clippy::expect_used)]

use serde_json::Value;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};
use sts2_harness::management::{
    AuthContext, ErrorClass, LiveWorkflowOptions, LiveWorkflowSession, LiveWorkflowSessionFactory,
    ManagementClient, ManagementError, ManagementServer, MemoryWorkflowStore, RunRequest,
    ServerConfig, StaticAuthenticator, SubmissionRecoveryState, TargetCatalogResponse,
    WorkflowStore,
};
use sts2_harness::workflow::WorkflowDefinition;

#[path = "support/live_workflow.rs"]
mod support;

struct BlockingOpenFactory {
    inner: support::FakeFactory,
    opens: AtomicUsize,
    release: Mutex<mpsc::Receiver<()>>,
}

impl BlockingOpenFactory {
    fn wait(&self) {
        self.opens.fetch_add(1, Ordering::SeqCst);
        self.release
            .lock()
            .expect("gate lock")
            .recv_timeout(Duration::from_secs(10))
            .expect("test releases open");
    }
}

impl LiveWorkflowSessionFactory for BlockingOpenFactory {
    fn capabilities(&self) -> Value {
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
        definition: &WorkflowDefinition,
        definition_digest: &str,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError> {
        self.wait();
        self.inner
            .open(request, actor, definition, definition_digest)
    }

    fn open_admitted(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &WorkflowDefinition,
        definition_digest: &str,
        control_limits: Option<&sts2_harness::management::ContextOwnerControlLimits>,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError> {
        self.wait();
        self.inner.open_admitted(
            request,
            actor,
            definition,
            definition_digest,
            control_limits,
        )
    }
}

struct ReleaseOnDrop(mpsc::Sender<()>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

#[test]
fn a_post_reservation_timeout_exposes_the_same_run_without_opening_another_session() {
    let store = Arc::new(MemoryWorkflowStore::new());
    let (sender, receiver) = mpsc::channel();
    let release = ReleaseOnDrop(sender);
    let factory = Arc::new(BlockingOpenFactory {
        inner: support::FakeFactory::new(false),
        opens: AtomicUsize::new(0),
        release: Mutex::new(receiver),
    });
    let service = Arc::new(
        support::live_service(
            store.clone(),
            factory.clone(),
            LiveWorkflowOptions::default(),
        )
        .expect("service"),
    );
    let auth = StaticAuthenticator::single("owner-token", support::actor()).expect("auth");
    let mut config = ServerConfig::new("127.0.0.1:0".parse().expect("address"), Arc::new(auth))
        .expect("server config");
    config.limits.dispatch_deadline = Duration::from_millis(100);
    config.limits.max_connections = 2;
    let server = ManagementServer::start(config, service.clone()).expect("server");
    let client = ManagementClient::new(server.address(), "owner-token").expect("client");
    let request = support::request("request-reserved-stall", support::definition(false));
    let body = serde_json::to_vec(&request).expect("body");
    let response = client
        .request_json("POST", "/v1/workflow-runs", Some(&body))
        .expect("timeout response");
    assert_eq!(response.status, 503);
    let error: Value = serde_json::from_slice(&response.body).expect("error");
    assert_eq!(error["error"]["class"], "write_state_unknown");
    assert_eq!(factory.opens.load(Ordering::SeqCst), 1);
    let pending = service
        .submission_recovery(&support::actor(), &request.request_id)
        .expect("recover reserved run");
    assert_eq!(pending.state, SubmissionRecoveryState::Pending);
    let run_id = pending.workflow_run_id.expect("run is already durable");
    assert!(store.get_run(&run_id).expect("lookup").is_some());
    let restricted = AuthContext::with_run_prefix(
        "operator",
        ["workflow:*".to_owned()],
        Some("different-run".to_owned()),
    )
    .expect("same subject with a restricted run prefix");
    assert_eq!(
        service
            .submit_run(&restricted, request.clone())
            .expect_err("pending replay must enforce the retained run prefix")
            .code,
        "run_scope_denied"
    );
    let no_live = AuthContext::new("operator", ["workflow:control".to_owned()])
        .expect("same subject without live scope");
    assert_eq!(
        service
            .submit_run(&no_live, request.clone())
            .expect_err("pending replay must enforce current live scope")
            .code,
        "live_scope_required"
    );
    assert_eq!(
        service
            .submit_run(&support::actor(), request.clone())
            .expect_err("must not open twice")
            .class,
        ErrorClass::WriteStateUnknown
    );
    assert_eq!(factory.opens.load(Ordering::SeqCst), 1);
    drop(release);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let outcome = service
            .submission_recovery(&support::actor(), &request.request_id)
            .expect("recover completion");
        if outcome.state == SubmissionRecoveryState::Submitted {
            assert_eq!(outcome.workflow_run_id.as_deref(), Some(run_id.as_str()));
            assert_eq!(
                outcome
                    .response
                    .expect("original submission")
                    .workflow_run_id,
                run_id
            );
            break;
        }
        assert!(Instant::now() < deadline, "late outcome must persist");
        std::thread::yield_now();
    }
    assert_eq!(factory.opens.load(Ordering::SeqCst), 1);
    assert_eq!(factory.inner.entries(), ["launch"]);
    server
        .shutdown()
        .expect("bounded worker joins after settling");
}
