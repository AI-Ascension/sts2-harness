// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;
use crate::management::workflow_ports::SyntheticDefinitionPort;
use crate::management::{
    AuthContext, DefinitionPort, DiffResult, InspectionResult, LiveWorkflowOptions,
    LiveWorkflowSession, LiveWorkflowSessionFactory, SqliteWorkflowStore, StaticAuthenticator,
    TargetCatalogResponse, ValidationResult,
};
use crate::workflow::WorkflowDefinition;
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

#[path = "../../tests/support/live_workflow.rs"]
pub(super) mod live;

type FixtureService = (Arc<ManagementService>, Arc<dyn Authenticator>);

pub(super) fn compose(root: &Path) -> Result<FixtureService, Box<dyn std::error::Error>> {
    record(root, "backend", &std::process::id().to_string());
    let store = Arc::new(SqliteWorkflowStore::open(root.join("workflow.db"))?);
    let factory = Arc::new(Factory {
        root: root.to_owned(),
        inner: live::FakeFactory::new(false),
    });
    let service = live::live_service(store, factory, LiveWorkflowOptions::default())?
        .with_definition_port(Arc::new(Definitions(root.to_owned())));
    let foreign = AuthContext::new("foreign-operator", ["workflow:*".to_owned()])?;
    let auth = StaticAuthenticator::single(TOKEN, live::actor())?
        .with_credential("foreign-owned-token", foreign)?;
    Ok((Arc::new(service), Arc::new(auth)))
}

pub(super) fn record(root: &Path, name: &str, value: &str) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join(name))
        .expect("record");
    file.write_all(format!("{value}\n").as_bytes())
        .expect("record value");
    file.sync_all().expect("durable test witness");
}

pub(super) fn never_return(root: &Path, name: &str) -> ! {
    record(root, name, &std::process::id().to_string());
    let mut command = entry_command(root);
    command
        .env(LEAF, "1")
        .env_remove(owned_backend::BACKEND_ROLE)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    let mut child = command.spawn().expect("owned descendant");
    record(root, "descendant", &child.id().to_string());
    // This fake port owns a permanent child wait, never reads cancellation or IPC,
    // and never runs a destructor. If the child unexpectedly exits, it still cannot
    // return. Only termination of the backend and descendant scope can finish it.
    let _ = child.wait();
    loop {
        std::thread::park();
    }
}

struct Definitions(PathBuf);
impl DefinitionPort for Definitions {
    fn validate(
        &self,
        definition: &Value,
        capabilities: &Value,
    ) -> Result<ValidationResult, ManagementError> {
        if definition["annotations"]["summary"].as_str() == Some("owned test stall") {
            never_return(&self.0, "definition-stall");
        }
        SyntheticDefinitionPort.validate(definition, capabilities)
    }
    fn inspect(&self, definition: &Value) -> Result<InspectionResult, ManagementError> {
        SyntheticDefinitionPort.inspect(definition)
    }
    fn diff(&self, before: &Value, after: &Value) -> Result<DiffResult, ManagementError> {
        SyntheticDefinitionPort.diff(before, after)
    }
}

struct Factory {
    root: PathBuf,
    inner: live::FakeFactory,
}
impl LiveWorkflowSessionFactory for Factory {
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
        record(&self.root, "opens", &request.request_id);
        if request.request_id.starts_with("stall-") {
            never_return(&self.root, "execution-stall");
        }
        self.inner
            .open(request, actor, definition, definition_digest)
    }
    fn open_admitted(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &WorkflowDefinition,
        definition_digest: &str,
        limits: Option<&crate::management::ContextOwnerControlLimits>,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError> {
        if limits.is_some_and(|limits| limits.max_control_events == 0) {
            return Err(ManagementError::capability(
                "context_control_event_limit_invalid",
                "invalid test limit",
            ));
        }
        self.open(request, actor, definition, definition_digest)
    }
}

pub(super) fn recorded(root: &Path, name: &str) -> Vec<String> {
    fs::read_to_string(root.join(name))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}
