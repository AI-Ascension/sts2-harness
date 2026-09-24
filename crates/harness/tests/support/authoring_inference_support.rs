// SPDX-License-Identifier: MIT

//! Synthetic doubles for the proposal-only authoring-inference suite (#105).
//!
//! Nothing here contacts a provider, model, credential or native host. The
//! provider double records that it was reached; the execution double records
//! that a run was *not* reached.

#![allow(clippy::expect_used)]
#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use sts2_harness::management::{
    AuthContext, AuthoringInferenceCandidate, AuthoringInferenceCost, AuthoringInferencePort,
    AuthoringInferenceProviderRequest, CapabilityPort, CommandApplication, CommandContext,
    DefinitionPort, Diagnostic, DiagnosticSeverity, DiffResult, InferenceProfileCatalog,
    InspectionResult, ManagementError, RunAdmission, RunRequest, ValidationResult,
    WorkflowExecutionPort, digest_value,
};
use sts2_harness::workflow::WORKFLOW_COMPILER_ID;

/// The owner capabilities value the endpoint digests into the manifest binding.
pub(crate) fn capabilities() -> Value {
    json!({
        "capabilities": ["workflow.live", "live.workflow.v1"],
        "owner": "synthetic.authoring.owner"
    })
}

/// Serves one fixed catalog (and the shared capabilities value) to the endpoint.
pub(crate) struct AuthoringCapabilityDouble {
    pub(crate) catalog: InferenceProfileCatalog,
}

impl CapabilityPort for AuthoringCapabilityDouble {
    fn capabilities(&self) -> Result<Value, ManagementError> {
        Ok(capabilities())
    }

    fn inference_profile_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<Option<InferenceProfileCatalog>, ManagementError> {
        Ok(Some(self.catalog.clone()))
    }
}

/// Accepts every candidate the owner compiler accepts, and refuses one explicit
/// marker so the owner-validation branch can be exercised.
pub(crate) struct AuthoringDefinitionDouble;

impl DefinitionPort for AuthoringDefinitionDouble {
    fn validate(
        &self,
        definition: &Value,
        _capabilities: &Value,
    ) -> Result<ValidationResult, ManagementError> {
        let refused = definition
            .pointer("/annotations/summary")
            .and_then(Value::as_str)
            == Some("owner-refused");
        let diagnostics = if refused {
            vec![Diagnostic {
                code: "capability_unknown".to_owned(),
                severity: DiagnosticSeverity::Error,
                path: "$.capabilities".to_owned(),
                message: "the owner refuses this candidate".to_owned(),
            }]
        } else {
            Vec::new()
        };
        Ok(ValidationResult {
            definition_digest: digest_value(definition)?,
            compiler: WORKFLOW_COMPILER_ID.to_owned(),
            diagnostics,
        })
    }

    fn inspect(&self, _definition: &Value) -> Result<InspectionResult, ManagementError> {
        Err(ManagementError::unavailable(
            "unused",
            "inspection is not used by this suite",
        ))
    }

    fn diff(&self, _old: &Value, _new: &Value) -> Result<DiffResult, ManagementError> {
        Err(ManagementError::unavailable(
            "unused",
            "diff is not used by this suite",
        ))
    }
}

/// Records that a run was never launched from the proposal-only endpoint.
pub(crate) struct RecordingExecutionPort {
    pub(crate) submissions: Arc<AtomicUsize>,
}

impl RecordingExecutionPort {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            submissions: Arc::new(AtomicUsize::new(0)),
        })
    }
}

impl WorkflowExecutionPort for RecordingExecutionPort {
    fn submit(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        _definition_digest: &str,
    ) -> Result<RunAdmission, ManagementError> {
        self.submissions.fetch_add(1, Ordering::SeqCst);
        Err(ManagementError::unavailable(
            "run_not_attached",
            "runs are not attached to this proposal-only owner",
        ))
    }

    fn apply_command(
        &self,
        _context: CommandContext,
    ) -> Result<CommandApplication, ManagementError> {
        Err(ManagementError::unavailable(
            "unused",
            "commands are not used by this suite",
        ))
    }
}

/// The provider's scripted outcome.
enum Planned {
    Candidate(AuthoringInferenceCandidate),
    Failure(ManagementError),
}

/// A recording fake authoring-inference provider.
pub(crate) struct RecordingAuthoringProvider {
    calls: Arc<AtomicUsize>,
    plan: Mutex<Planned>,
    hook: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl RecordingAuthoringProvider {
    pub(crate) fn returning(candidate: AuthoringInferenceCandidate) -> Arc<Self> {
        Arc::new(Self {
            calls: Arc::new(AtomicUsize::new(0)),
            plan: Mutex::new(Planned::Candidate(candidate)),
            hook: Mutex::new(None),
        })
    }

    pub(crate) fn failing(error: ManagementError) -> Arc<Self> {
        Arc::new(Self {
            calls: Arc::new(AtomicUsize::new(0)),
            plan: Mutex::new(Planned::Failure(error)),
            hook: Mutex::new(None),
        })
    }

    /// Runs `hook` synchronously on every provider call, before the candidate is
    /// returned, so a test can model a concurrent draft edit during generation.
    pub(crate) fn on_call(&self, hook: impl Fn() + Send + Sync + 'static) {
        *self.hook.lock().expect("hook lock") = Some(Box::new(hook));
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl AuthoringInferencePort for RecordingAuthoringProvider {
    fn propose(
        &self,
        _request: &AuthoringInferenceProviderRequest,
    ) -> Result<AuthoringInferenceCandidate, ManagementError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = self.hook.lock().expect("hook lock").as_ref() {
            hook();
        }
        match &*self.plan.lock().expect("plan lock") {
            Planned::Candidate(candidate) => Ok(candidate.clone()),
            Planned::Failure(error) => Err(error.clone()),
        }
    }
}

/// Builds a candidate with an honest cost and no diagnostics.
pub(crate) fn candidate(
    definition: Value,
    provider_calls: u64,
    output_tokens: u64,
) -> AuthoringInferenceCandidate {
    AuthoringInferenceCandidate {
        definition,
        unsatisfied: Vec::new(),
        diagnostics: Vec::new(),
        cost: AuthoringInferenceCost {
            provider_calls,
            output_tokens,
        },
    }
}
