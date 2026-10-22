// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde_json::Value;
use std::sync::Condvar;

use super::*;
use crate::management::workflow_ports::SyntheticDefinitionPort;
use crate::management::{DefinitionPort, DiffResult, InspectionResult, ValidationResult};

#[derive(Default)]
pub(super) struct Stall {
    pub entered: AtomicUsize,
    released: Mutex<bool>,
    wake: Condvar,
}

impl Stall {
    fn wait(&self) {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let guard = self.released.lock().expect("stall lock");
        let (guard, timeout) = self
            .wake
            .wait_timeout_while(guard, Duration::from_secs(10), |released| !*released)
            .expect("stall condition");
        drop(guard);
        assert!(!timeout.timed_out(), "test must release its stalled port");
    }

    pub(super) fn release(&self) {
        *self.released.lock().expect("stall lock") = true;
        self.wake.notify_all();
    }
}

/// Releases on assertion failure too, so a failed test cannot leak its worker.
pub(super) struct ReleaseOnDrop(pub Arc<Stall>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

pub(super) struct StalledDefinition(pub Arc<Stall>);
impl DefinitionPort for StalledDefinition {
    fn validate(
        &self,
        definition: &Value,
        capabilities: &Value,
    ) -> Result<ValidationResult, ManagementError> {
        self.0.wait();
        SyntheticDefinitionPort.validate(definition, capabilities)
    }
    fn inspect(&self, definition: &Value) -> Result<InspectionResult, ManagementError> {
        SyntheticDefinitionPort.inspect(definition)
    }
    fn diff(&self, before: &Value, after: &Value) -> Result<DiffResult, ManagementError> {
        SyntheticDefinitionPort.diff(before, after)
    }
}

#[derive(Debug, Default)]
pub(super) struct Lines(pub Mutex<Vec<String>>);
impl super::super::lifecycle_log::LifecycleLogSink for Lines {
    fn wall_clock(&self) -> Option<(u64, u32)> {
        Some((1, 0))
    }
    fn monotonic_millis(&self) -> u64 {
        0
    }
    fn write_line(&self, line: &str) {
        self.0.lock().expect("log").push(line.to_owned());
    }
}

pub(super) fn body(request_id: &str) -> Vec<u8> {
    let definition: Value = serde_json::from_slice(include_bytes!(
        "../../../../conformance/workflow-v1/valid-strict.json"
    ))
    .expect("synthetic definition");
    serde_json::to_vec(&RunRequest {
        schema_version: crate::management::MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: request_id.to_owned(),
        definition: Some(definition),
        artifact_id: None,
        instance_id: "fixture-instance".to_owned(),
        profile: "synthetic".to_owned(),
        admission: None,
    })
    .expect("request")
}

pub(super) fn json(response: ClientResponse) -> Value {
    serde_json::from_slice(&response.body).expect("JSON response")
}
