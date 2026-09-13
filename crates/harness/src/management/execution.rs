// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::super::auth::AuthContext;
use super::super::contract::{
    CommandKind, CommandOutcome, EventClassification, EventPayload, EventType, RunEvent,
    RunRequest, WorkflowRunStatus,
};
use super::super::service::{
    CommandApplication, CommandContext, ManagementError, RunAdmission, WorkflowExecutionPort,
};
use super::session::{LiveWorkflowOptions, LiveWorkflowSession, LiveWorkflowSessionFactory};
use crate::episode::{
    ActionIdentity, EpisodeLegalAction, EpisodeLegalActionSet, EpisodeObservation,
    TransitionReceipt,
};
use crate::workflow::{CompiledWorkflow, RuntimeFault, RuntimeStatus, StrictRuntime};

use super::execution_records::{
    application, live_run_id, lock_error, management_status, runtime_error, snapshot_from_runtime,
};
use super::node::{LiveNodeExecutor, reconcile_pending};

pub(super) struct LiveRun {
    pub(super) runtime: StrictRuntime,
    pub(super) definition_digest: String,
    pub(super) run_id: String,
    pub(super) instance_id: String,
    pub(super) state: LiveNodeState,
    pub(super) cancelled: bool,
}

pub(super) struct LiveNodeState {
    pub(super) session: Box<dyn LiveWorkflowSession>,
    pub(super) observation: Option<EpisodeObservation>,
    pub(super) actions: Option<EpisodeLegalActionSet>,
    pub(super) pending: Option<PendingDispatch>,
    pub(super) provider_calls: u64,
    pub(super) max_provider_calls: u64,
    pub(super) options: LiveWorkflowOptions,
}

pub(super) struct PendingDispatch {
    pub(super) identity: ActionIdentity,
    pub(super) action: EpisodeLegalAction,
    pub(super) state: super::super::contract::PendingOperationState,
    pub(super) resolved: Option<TransitionReceipt>,
}

/// Live execution keeps the authored graph and operation identity in one
/// bounded in-memory coordinator. Missing state after restart fails closed.
pub struct LiveWorkflowExecutionPort {
    factory: Arc<dyn LiveWorkflowSessionFactory>,
    options: LiveWorkflowOptions,
    runs: Mutex<BTreeMap<String, LiveRun>>,
}

impl LiveWorkflowExecutionPort {
    pub fn new(
        factory: Arc<dyn LiveWorkflowSessionFactory>,
        options: LiveWorkflowOptions,
    ) -> Result<Self, ManagementError> {
        options.validate()?;
        Ok(Self {
            factory,
            options,
            runs: Mutex::new(BTreeMap::new()),
        })
    }
}

impl WorkflowExecutionPort for LiveWorkflowExecutionPort {
    fn submit(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition_digest: &str,
    ) -> Result<RunAdmission, ManagementError> {
        let value = request.definition.as_ref().ok_or_else(|| {
            ManagementError::unavailable(
                "artifact_port_unavailable",
                "live execution requires an admitted workflow definition",
            )
        })?;
        let definition = super::super::workflow_ports::parse_definition(value)?;
        if definition
            .annotations
            .as_ref()
            .is_some_and(|item| item.synthetic)
        {
            return Err(ManagementError::capability(
                "synthetic_definition_not_live",
                "synthetic workflow definitions cannot enter the live execution port",
            ));
        }
        let digest = super::super::workflow_ports::raw_digest(value)?;
        if digest != definition_digest {
            return Err(ManagementError::conflict(
                "live_identity_mismatch",
                "live execution is bound to a different definition digest",
            ));
        }
        let compiled = CompiledWorkflow::compile(definition.clone())
            .map_err(|error| ManagementError::invalid("definition_compile", error.to_string()))?;
        let mut session = self
            .factory
            .open(request, actor, &definition, definition_digest)?;
        let runtime = StrictRuntime::new(compiled)
            .map_err(|error| ManagementError::invalid("runtime_admission", error.to_string()))?;
        session.launch()?;
        let run_id = live_run_id(request, definition_digest)?;
        let snapshot = snapshot_from_runtime(
            &run_id,
            &request.instance_id,
            definition_digest,
            &runtime,
            false,
            None,
            1,
        );
        let event = RunEvent {
            schema_version: super::super::contract::EVENT_SCHEMA_VERSION.to_owned(),
            workflow_run_id: run_id.clone(),
            sequence: 1,
            run_revision: 1,
            event_type: EventType::RunStarted,
            definition_digest: definition_digest.to_owned(),
            node_execution_id: snapshot.cursor.node_execution_id.clone(),
            payload: EventPayload {
                operation_id: None,
                classification: Some(EventClassification::Accepted),
                reason_code: "live_admitted".to_owned(),
            },
            integrity_digest: None,
        };
        let run = LiveRun {
            runtime,
            definition_digest: definition_digest.to_owned(),
            run_id: run_id.clone(),
            instance_id: request.instance_id.clone(),
            state: LiveNodeState {
                session,
                observation: None,
                actions: None,
                pending: None,
                provider_calls: 0,
                max_provider_calls: definition.limits.max_provider_calls,
                options: self.options.clone(),
            },
            cancelled: false,
        };
        let mut runs = self.runs.lock().map_err(lock_error)?;
        if runs.insert(run_id, run).is_some() {
            return Err(ManagementError::conflict(
                "live_duplicate_run",
                "live run identity was already admitted",
            ));
        }
        Ok(RunAdmission {
            snapshot,
            initial_events: vec![event],
        })
    }

    fn apply_command(
        &self,
        context: CommandContext,
    ) -> Result<CommandApplication, ManagementError> {
        let mut runs = self.runs.lock().map_err(lock_error)?;
        let run = runs.get_mut(&context.request.run_id).ok_or_else(|| {
            ManagementError::unresolved(
                "live_runtime_after_restart",
                "live session is unavailable after service restart",
            )
        })?;
        if run.definition_digest != context.snapshot.definition_digest {
            return Err(ManagementError::conflict(
                "live_identity_mismatch",
                "live command is bound to a different workflow definition",
            ));
        }
        let revision = context.snapshot.run_revision.saturating_add(1);
        let (status, outcome, reason) = match context.request.kind {
            CommandKind::Pause => {
                if run.cancelled {
                    return Ok(application(
                        run,
                        WorkflowRunStatus::Cancelled,
                        CommandOutcome::Applied,
                        "cancel_dominates",
                        revision,
                    ));
                }
                run.state.session.pause()?;
                run.runtime.pause().map_err(runtime_error)?;
                (WorkflowRunStatus::Paused, CommandOutcome::Applied, "pause")
            }
            CommandKind::Resume => {
                if run.cancelled {
                    (
                        WorkflowRunStatus::Cancelled,
                        CommandOutcome::Applied,
                        "cancel_dominates",
                    )
                } else {
                    run.state.session.resume()?;
                    run.runtime.resume().map_err(runtime_error)?;
                    (
                        WorkflowRunStatus::Running,
                        CommandOutcome::Applied,
                        "resume",
                    )
                }
            }
            CommandKind::Cancel => {
                if let Err(error) = reconcile_pending(&mut run.state)
                    && error.class != super::super::contract::ErrorClass::Unresolved
                {
                    return Err(error);
                }
                run.state.session.stop_episode()?;
                run.cancelled = true;
                (
                    WorkflowRunStatus::Cancelled,
                    CommandOutcome::Applied,
                    "cancel",
                )
            }
            CommandKind::Step => {
                if run.cancelled {
                    (
                        WorkflowRunStatus::Cancelled,
                        CommandOutcome::Applied,
                        "cancel_dominates",
                    )
                } else if run.runtime.status() == RuntimeStatus::Paused {
                    return Err(ManagementError::conflict(
                        "live_run_paused",
                        "step requires a resumed workflow",
                    ));
                } else {
                    reconcile_pending(&mut run.state)?;
                    if run.runtime.status() == RuntimeStatus::NeedsOperator {
                        run.runtime
                            .resume_after_unknown_effect()
                            .map_err(runtime_error)?;
                    }
                    let mut executor = LiveNodeExecutor {
                        state: &mut run.state,
                    };
                    match run.runtime.step(&mut executor) {
                        Ok(status) => (
                            management_status(status, false),
                            CommandOutcome::Applied,
                            "step",
                        ),
                        Err(RuntimeFault::UnknownEffect) => {
                            let application = application(
                                run,
                                WorkflowRunStatus::NeedsOperator,
                                CommandOutcome::Pending,
                                "live_operation_unknown",
                                revision,
                            );
                            return Ok(application);
                        }
                        Err(error) => return Err(runtime_error(error)),
                    }
                }
            }
        };
        Ok(application(run, status, outcome, reason, revision))
    }
}
