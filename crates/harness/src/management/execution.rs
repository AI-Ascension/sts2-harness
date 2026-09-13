// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, btree_map::Entry};
use std::sync::{Arc, Mutex};

use super::super::auth::AuthContext;
use super::super::contract::{
    EventClassification, EventPayload, EventType, PendingOperation, RunEvent, RunRequest,
};
use super::super::service::{
    CommandApplication, CommandContext, ManagementError, RunAdmission, WorkflowExecutionPort,
};
use super::session::{LiveWorkflowOptions, LiveWorkflowSession, LiveWorkflowSessionFactory};
use crate::episode::{
    ActionIdentity, EpisodeLegalAction, EpisodeLegalActionSet, EpisodeObservation,
    TransitionReceipt,
};
use crate::workflow::{CompiledWorkflow, StrictRuntime};

use super::execution_records::{
    SnapshotState, cleanup_session, live_run_id, lock_error, snapshot_from_runtime,
};

pub(super) struct LiveRun {
    pub(super) runtime: StrictRuntime,
    pub(super) definition_digest: String,
    pub(super) run_id: String,
    pub(super) instance_id: String,
    pub(super) state: LiveNodeState,
    pub(super) cancelled: bool,
    pub(super) cleanup: super::super::contract::CleanupState,
}

pub(super) struct LiveNodeState {
    pub(super) session: Box<dyn LiveWorkflowSession>,
    pub(super) instance_id: String,
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

    pub(super) fn runs(&self) -> &Mutex<BTreeMap<String, LiveRun>> {
        &self.runs
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
        if request.profile != "live" && !request.profile.starts_with("live.") {
            return Err(ManagementError::capability(
                "live_profile_required",
                "live execution requires an explicit live.* workflow profile",
            ));
        }
        let definition = super::super::workflow_ports::parse_definition(value)?;
        if definition
            .annotations
            .as_ref()
            .is_some_and(|item| item.synthetic)
            || definition.game_profile.as_str().contains("synthetic")
            || definition.policy_ref.as_str().contains("synthetic")
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
        if let Err(error) = session.launch() {
            let stop_error = session.stop_episode().err();
            let release_error = session.release_lease().err();
            if let Some(cleanup_error) = stop_error.or(release_error) {
                return Err(ManagementError::unavailable(
                    "live_launch_cleanup_failed",
                    format!("live launch failed ({error}); cleanup failed ({cleanup_error})"),
                ));
            }
            return Err(error);
        }
        let run_id = live_run_id(request, definition_digest)?;
        let snapshot = snapshot_from_runtime(
            &run_id,
            definition_digest,
            &runtime,
            SnapshotState::default(),
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
                instance_id: request.instance_id.clone(),
                observation: None,
                actions: None,
                pending: None,
                provider_calls: 0,
                max_provider_calls: definition.limits.max_provider_calls,
                options: self.options.clone(),
            },
            cancelled: false,
            cleanup: super::super::contract::CleanupState::NotStarted,
        };
        let mut runs = self.runs.lock().map_err(lock_error)?;
        match runs.entry(run_id) {
            Entry::Vacant(entry) => {
                entry.insert(run);
            }
            Entry::Occupied(_) => {
                let mut run = run;
                let _ = cleanup_session(&mut run, true);
                return Err(ManagementError::conflict(
                    "live_duplicate_run",
                    "live run identity was already admitted",
                ));
            }
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
        super::execution_commands::apply_command(self, context, None)
    }

    fn apply_command_with_intent(
        &self,
        context: CommandContext,
        record_intent: &dyn Fn(PendingOperation) -> Result<(), ManagementError>,
    ) -> Result<CommandApplication, ManagementError> {
        super::execution_commands::apply_command(self, context, Some(record_intent))
    }
}
