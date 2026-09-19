// SPDX-License-Identifier: MIT

//! Recording process-lifecycle port plus an admitted live-run fixture.
//!
//! Every type here is a synthetic double. The gateway advertises
//! `available: false` today because no concrete OS adapter exists (ADR 0024),
//! so these tests prove harness-side ordering, identity, and durability rather
//! than native launch, stop, or attach behaviour. The recording port is the
//! downstream boundary: counting and inspecting its calls is how a test shows
//! an effect did or did not happen.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use sts2_harness::management::{
    AuthContext, LaunchProfileId, LifecycleAction, LifecycleCommand, LifecycleIntentStore,
    LifecycleOperationState, LifecycleOperationView, LifecycleState, LifecycleTarget,
    LiveWorkflowOptions, LiveWorkflowSessionFactory, ManagementError, ManagementService,
    PROCESS_LIFECYCLE_CONTRACT, ProcessLifecycleCapability, ProcessLifecyclePort,
    SqliteWorkflowStore, WorkflowStore, digest_value, live_run_id,
};

#[path = "live_workflow.rs"]
mod live_workflow;

#[path = "process_lifecycle_commands.rs"]
mod commands;

pub(crate) use live_workflow::{actor, definition, request};

pub(crate) use commands::{command_body, error_code, http, launch, stop, unadmitted};

/// The instance every admitted run in this file targets.
pub(crate) const INSTANCE: &str = "instance-1";
/// An instance no run in this file is admitted to.
pub(crate) const FOREIGN_INSTANCE: &str = "instance-2";
/// The authority epoch used by the recording answers.
pub(crate) const EPOCH: u64 = 3;
/// Schema of the harness-owned lifecycle command envelope.
pub(crate) const COMMAND_SCHEMA: &str = "ascension.process-lifecycle-command/v1";

/// Error class a recording answer should produce.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Class {
    Invalid,
    Forbidden,
    Conflict,
    Unavailable,
}

/// One scripted gateway answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Answer {
    /// A settled operation answer in the given states.
    Operation(LifecycleOperationState, LifecycleState),
    /// A typed refusal or a transport failure.
    Error { class: Class, code: &'static str },
}

impl Answer {
    /// A launch acknowledgement: accepted and starting, explicitly not ready.
    pub(crate) const STARTING: Self =
        Self::Operation(LifecycleOperationState::Starting, LifecycleState::Starting);
    /// A settled stop.
    pub(crate) const STOPPED: Self =
        Self::Operation(LifecycleOperationState::Stopped, LifecycleState::Stopped);
    /// A stop the gateway could not settle, so the outcome stays unknown.
    pub(crate) const BLOCKED: Self =
        Self::Operation(LifecycleOperationState::Blocked, LifecycleState::Degraded);
    /// The gateway answered nothing: the effect may still have happened.
    pub(crate) const TRANSPORT: Self = Self::Error {
        class: Class::Unavailable,
        code: "process_lifecycle_transport",
    };
}

/// One recorded submission, in authored order.
#[derive(Clone, Debug)]
pub(crate) struct SubmitRecord {
    pub(crate) command_id: String,
    pub(crate) operation_id: u64,
    pub(crate) authority_epoch: u64,
    pub(crate) instance_id: String,
    pub(crate) action: LifecycleAction,
}

struct Inner {
    target_instance: String,
    capability_instance: String,
    available: bool,
    profiles: Vec<LaunchProfileId>,
    script: Vec<Answer>,
    submits: Vec<SubmitRecord>,
    lookups: Vec<u64>,
    lookup_answer: Answer,
    epochs: BTreeMap<u64, u64>,
    seen_targets: Vec<String>,
}

/// A `ProcessLifecyclePort` that records what it was asked to do.
pub(crate) struct RecordingPort {
    inner: Mutex<Inner>,
}

impl RecordingPort {
    /// Builds a port configured for `INSTANCE`, answering submissions from
    /// `script` in order and any lookup with `lookup_answer`.
    pub(crate) fn new(script: Vec<Answer>, lookup_answer: Answer) -> Self {
        Self {
            inner: Mutex::new(Inner {
                target_instance: INSTANCE.to_owned(),
                capability_instance: INSTANCE.to_owned(),
                available: false,
                profiles: vec![LaunchProfileId::new(11).expect("profile 11")],
                script,
                submits: Vec::new(),
                lookups: Vec::new(),
                lookup_answer,
                epochs: BTreeMap::new(),
                seen_targets: Vec::new(),
            }),
        }
    }

    /// Advertises `instance` as the capability's instance.
    pub(crate) fn advertise(&self, instance: &str) {
        self.inner.lock().expect("inner").capability_instance = instance.to_owned();
    }

    /// Every submission the port received, in order.
    pub(crate) fn submits(&self) -> Vec<SubmitRecord> {
        self.inner.lock().expect("inner").submits.clone()
    }

    /// Every operation identity a lookup was issued for, in order.
    pub(crate) fn lookups(&self) -> Vec<u64> {
        self.inner.lock().expect("inner").lookups.clone()
    }

    /// Every instance the harness asked this port to act on, in order.
    pub(crate) fn seen_targets(&self) -> Vec<String> {
        self.inner.lock().expect("inner").seen_targets.clone()
    }

    fn refuse_foreign(&self, inner: &mut Inner, target: &LifecycleTarget) -> bool {
        inner.seen_targets.push(target.instance_id.clone());
        target.instance_id != inner.target_instance
    }

    fn answer(
        inner: &Inner,
        answer: Answer,
        operation_id: u64,
        authority_epoch: u64,
    ) -> Result<LifecycleOperationView, ManagementError> {
        match answer {
            Answer::Operation(operation_state, state) => Ok(LifecycleOperationView {
                contract: PROCESS_LIFECYCLE_CONTRACT.to_owned(),
                operation_id,
                instance_id: inner.target_instance.clone(),
                state,
                operation_state,
                process: None,
                authority_epoch,
                failure: None,
            }),
            Answer::Error { class, code } => Err(match class {
                Class::Invalid => ManagementError::invalid(code, "scripted refusal"),
                Class::Forbidden => ManagementError::forbidden(code, "scripted refusal"),
                Class::Conflict => ManagementError::conflict(code, "scripted refusal"),
                Class::Unavailable => ManagementError::unavailable(code, "scripted transport"),
            }),
        }
    }
}

impl ProcessLifecyclePort for RecordingPort {
    fn capability(
        &self,
        _actor: &AuthContext,
        target: &LifecycleTarget,
    ) -> Result<ProcessLifecycleCapability, ManagementError> {
        let mut inner = self.inner.lock().expect("inner");
        if self.refuse_foreign(&mut inner, target) {
            return Err(ManagementError::forbidden(
                "process_lifecycle_unowned_target",
                "the configured gateway is not the target instance",
            ));
        }
        Ok(ProcessLifecycleCapability {
            contract: PROCESS_LIFECYCLE_CONTRACT.to_owned(),
            available: inner.available,
            profiles: inner.profiles.clone(),
            authority_epoch: Some(EPOCH),
            instance_id: inner.capability_instance.clone(),
            unavailable_reason: Some("no concrete lifecycle adapter is configured".to_owned()),
        })
    }

    fn submit(
        &self,
        _actor: &AuthContext,
        target: &LifecycleTarget,
        command: &LifecycleCommand,
        action: &LifecycleAction,
    ) -> Result<LifecycleOperationView, ManagementError> {
        let mut inner = self.inner.lock().expect("inner");
        if self.refuse_foreign(&mut inner, target) {
            return Err(ManagementError::forbidden(
                "process_lifecycle_unowned_target",
                "the configured gateway is not the target instance",
            ));
        }
        inner
            .epochs
            .insert(command.operation_id, command.authority_epoch);
        inner.submits.push(SubmitRecord {
            command_id: command.command_id.clone(),
            operation_id: command.operation_id,
            authority_epoch: command.authority_epoch,
            instance_id: target.instance_id.clone(),
            action: action.clone(),
        });
        let index = inner.submits.len() - 1;
        let answer = inner.script.get(index).copied().unwrap_or(Answer::STARTING);
        Self::answer(
            &inner,
            answer,
            command.operation_id,
            command.authority_epoch,
        )
    }

    fn lookup(
        &self,
        _actor: &AuthContext,
        target: &LifecycleTarget,
        operation_id: u64,
    ) -> Result<LifecycleOperationView, ManagementError> {
        let mut inner = self.inner.lock().expect("inner");
        if self.refuse_foreign(&mut inner, target) {
            return Err(ManagementError::forbidden(
                "process_lifecycle_unowned_target",
                "the configured gateway is not the target instance",
            ));
        }
        inner.lookups.push(operation_id);
        let epoch = inner.epochs.get(&operation_id).copied().unwrap_or(EPOCH);
        let answer = inner.lookup_answer;
        Self::answer(&inner, answer, operation_id, epoch)
    }
}

/// An admitted live run served over the real management HTTP surface with a
/// recording lifecycle port attached.
pub(crate) struct Harness {
    pub(crate) service: Arc<ManagementService>,
    pub(crate) port: Arc<RecordingPort>,
    pub(crate) run_id: String,
    pub(crate) run_revision: u64,
    pub(crate) request_id: String,
    store_path: PathBuf,
    intent_dir: PathBuf,
}

impl Harness {
    /// Restarts the harness over the same workflow store and intent directory.
    ///
    /// A fresh in-process service plus a reopened durable intent store is the
    /// harness half of a restart: nothing durable is carried in memory. The
    /// previous service is dropped first, because the durable journal grants
    /// ownership of its file to exactly one in-process store at a time.
    #[must_use]
    pub(crate) fn restart(self) -> Self {
        let store_path = self.store_path.clone();
        let intent_dir = self.intent_dir.clone();
        let port = Arc::clone(&self.port);
        let request_id = self.request_id.clone();
        drop(self);
        assemble(&store_path, &intent_dir, port, request_id, false)
    }

    /// Issues one management HTTP request against this harness.
    pub(crate) fn http(&self, method: &str, path: &str, body: Option<&[u8]>) -> (u16, Value) {
        http(&self.service, method, path, body)
    }

    /// The lifecycle command route for this run.
    pub(crate) fn operations_path(&self) -> String {
        format!(
            "/v1/workflow-runs/{}/process-lifecycle/operations",
            self.run_id
        )
    }

    /// The capability route for this run.
    pub(crate) fn capability_path(&self) -> String {
        format!("/v1/workflow-runs/{}/process-lifecycle", self.run_id)
    }

    /// One lifecycle command body with the harness-authored identity fields.
    pub(crate) fn command(
        &self,
        command_id: &str,
        operation_id: u64,
        authority_epoch: u64,
        action: Value,
    ) -> Vec<u8> {
        command_body(
            command_id,
            &self.run_id,
            self.run_revision,
            operation_id,
            authority_epoch,
            action,
        )
    }
}

/// Builds the fixture: a private directory, an admitted live run, and the port.
pub(crate) fn harness(request_id: &str, script: Vec<Answer>, lookup_answer: Answer) -> Harness {
    let root = std::env::temp_dir().join(format!("sts2-lifecycle-http-{}", uuid::Uuid::new_v4()));
    commands::create_private_directory(&root).expect("private directory");
    assemble(
        &root.join("store.sqlite"),
        &root.join("intents"),
        Arc::new(RecordingPort::new(script, lookup_answer)),
        request_id.to_owned(),
        true,
    )
}

fn assemble(
    store_path: &Path,
    intent_dir: &Path,
    port: Arc<RecordingPort>,
    request_id: String,
    fresh: bool,
) -> Harness {
    let store: Arc<dyn WorkflowStore> =
        Arc::new(SqliteWorkflowStore::open(store_path).expect("workflow store"));
    let factory: Arc<dyn LiveWorkflowSessionFactory> =
        Arc::new(live_workflow::FakeFactory::new(false));
    let service =
        live_workflow::live_service(Arc::clone(&store), factory, LiveWorkflowOptions::default())
            .expect("live service");
    let intents = Arc::new(Mutex::new(
        LifecycleIntentStore::open(intent_dir).expect("intent store"),
    ));
    let service = service.with_process_lifecycle(port.clone(), intents);
    let request = request(&request_id, definition(false));
    let run_id =
        live_run_id(&request, &digest_value(&definition(false)).expect("digest")).expect("run id");
    // A restarted harness adopts the durable run instead of re-submitting it:
    // the live path refuses to re-run a submission whose session is gone.
    let run_revision = if fresh {
        let snapshot = service
            .submit_run(&actor(), request)
            .expect("admitted live run");
        assert_eq!(snapshot.workflow_run_id, run_id);
        assert!(
            store
                .get_run(&run_id)
                .expect("store read")
                .expect("run persisted")
                .admission
                .is_some(),
            "run must be admitted"
        );
        snapshot.run_revision
    } else {
        store
            .get_run(&run_id)
            .expect("store read")
            .expect("run retained across the restart")
            .run_revision
    };
    Harness {
        service: Arc::new(service),
        port,
        run_id,
        run_revision,
        request_id,
        store_path: store_path.to_path_buf(),
        intent_dir: intent_dir.to_path_buf(),
    }
}
