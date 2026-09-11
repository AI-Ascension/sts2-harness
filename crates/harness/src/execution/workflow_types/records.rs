// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::super::types::ExecutionStoreError;
use super::{
    GameOperationId, MAX_WORKFLOW_CURSOR, RunProjection, RunStatus, WorkflowCommandId,
    WorkflowDefinitionId, WorkflowEpisodeId, WorkflowEventId, WorkflowInvocationId, WorkflowPlanId,
    WorkflowRunId, digest_bytes, validate_bytes,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum InvocationOutcome {
    Accepted,
    Rejected,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum WorkflowEventPayload {
    RunStarted {
        workflow_id: WorkflowDefinitionId,
        plan_id: WorkflowPlanId,
        episode_id: WorkflowEpisodeId,
    },
    CursorAdvanced {
        cursor: u64,
        stack: Vec<String>,
        counters: BTreeMap<String, u64>,
    },
    InvocationReserved {
        invocation_id: WorkflowInvocationId,
        command_id: WorkflowCommandId,
        game_operation_id: Option<GameOperationId>,
    },
    InvocationCompleted {
        invocation_id: WorkflowInvocationId,
        outcome: InvocationOutcome,
    },
    RunCompleted,
    RunFailed,
}

impl WorkflowEventPayload {
    pub(crate) const fn kind(&self) -> &'static str {
        match self {
            Self::RunStarted { .. } => "run_started",
            Self::CursorAdvanced { .. } => "cursor_advanced",
            Self::InvocationReserved { .. } => "invocation_reserved",
            Self::InvocationCompleted { .. } => "invocation_completed",
            Self::RunCompleted => "run_completed",
            Self::RunFailed => "run_failed",
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ExecutionStoreError> {
        match self {
            Self::RunStarted {
                workflow_id,
                plan_id,
                episode_id,
            } => {
                workflow_id.validate()?;
                plan_id.validate()?;
                episode_id.validate()?;
            }
            Self::CursorAdvanced {
                cursor,
                stack,
                counters,
            } => {
                if *cursor > MAX_WORKFLOW_CURSOR {
                    return Err(ExecutionStoreError::InvalidWorkflowEvent);
                }
                RunProjection {
                    status: RunStatus::Running,
                    cursor: *cursor,
                    stack: stack.clone(),
                    counters: counters.clone(),
                }
                .validate()?;
            }
            Self::InvocationReserved {
                invocation_id,
                command_id,
                game_operation_id,
            } => {
                invocation_id.validate()?;
                command_id.validate()?;
                if let Some(operation_id) = game_operation_id {
                    operation_id.validate()?;
                }
            }
            Self::InvocationCompleted { invocation_id, .. } => invocation_id.validate()?,
            Self::RunCompleted | Self::RunFailed => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowRunStart {
    pub run_id: WorkflowRunId,
    pub workflow_id: WorkflowDefinitionId,
    pub plan_id: WorkflowPlanId,
    pub episode_id: WorkflowEpisodeId,
    pub initial_projection: RunProjection,
}

impl WorkflowRunStart {
    pub fn new(
        run_id: WorkflowRunId,
        workflow_id: WorkflowDefinitionId,
        plan_id: WorkflowPlanId,
        episode_id: WorkflowEpisodeId,
    ) -> Result<Self, ExecutionStoreError> {
        run_id.validate()?;
        workflow_id.validate()?;
        plan_id.validate()?;
        episode_id.validate()?;
        let initial_projection = RunProjection::new();
        initial_projection.validate()?;
        Ok(Self {
            run_id,
            workflow_id,
            plan_id,
            episode_id,
            initial_projection,
        })
    }

    pub fn validate(&self) -> Result<(), ExecutionStoreError> {
        self.run_id.validate()?;
        self.workflow_id.validate()?;
        self.plan_id.validate()?;
        self.episode_id.validate()?;
        self.initial_projection.validate()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowEvent {
    pub event_id: WorkflowEventId,
    pub run_id: WorkflowRunId,
    pub payload: WorkflowEventPayload,
}

impl WorkflowEvent {
    pub fn new(
        event_id: WorkflowEventId,
        run_id: WorkflowRunId,
        payload: WorkflowEventPayload,
    ) -> Result<Self, ExecutionStoreError> {
        event_id.validate()?;
        run_id.validate()?;
        payload.validate()?;
        Ok(Self {
            event_id,
            run_id,
            payload,
        })
    }

    pub fn validate(&self) -> Result<(), ExecutionStoreError> {
        self.event_id.validate()?;
        self.run_id.validate()?;
        self.payload.validate()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvocationState {
    Reserved,
    Sent,
    Accepted,
    Rejected,
    Unknown,
}

impl InvocationState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Sent => "sent",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Unknown => "unknown",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "reserved" => Self::Reserved,
            "sent" => Self::Sent,
            "accepted" => Self::Accepted,
            "rejected" => Self::Rejected,
            "unknown" => Self::Unknown,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowInvocation {
    pub invocation_id: WorkflowInvocationId,
    pub run_id: WorkflowRunId,
    pub episode_id: WorkflowEpisodeId,
    pub plan_id: WorkflowPlanId,
    pub command_id: WorkflowCommandId,
    pub game_operation_id: Option<GameOperationId>,
    pub payload: Vec<u8>,
    pub payload_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredWorkflowInvocation {
    pub invocation_id: WorkflowInvocationId,
    pub run_id: WorkflowRunId,
    pub episode_id: WorkflowEpisodeId,
    pub plan_id: WorkflowPlanId,
    pub command_id: WorkflowCommandId,
    pub game_operation_id: Option<GameOperationId>,
    pub payload_digest: String,
    pub state: InvocationState,
    pub revision: u64,
    pub send_marker: bool,
}

impl WorkflowInvocation {
    pub fn new(
        invocation_id: WorkflowInvocationId,
        run_id: WorkflowRunId,
        episode_id: WorkflowEpisodeId,
        plan_id: WorkflowPlanId,
        command_id: WorkflowCommandId,
        game_operation_id: Option<GameOperationId>,
        payload: Vec<u8>,
    ) -> Result<Self, ExecutionStoreError> {
        invocation_id.validate()?;
        run_id.validate()?;
        episode_id.validate()?;
        plan_id.validate()?;
        command_id.validate()?;
        if let Some(operation_id) = &game_operation_id {
            operation_id.validate()?;
        }
        validate_bytes(&payload)?;
        Ok(Self {
            invocation_id,
            run_id,
            episode_id,
            plan_id,
            command_id,
            game_operation_id,
            payload_digest: digest_bytes(&payload),
            payload,
        })
    }

    pub fn validate(&self) -> Result<(), ExecutionStoreError> {
        self.invocation_id.validate()?;
        self.run_id.validate()?;
        self.episode_id.validate()?;
        self.plan_id.validate()?;
        self.command_id.validate()?;
        if let Some(operation_id) = &self.game_operation_id {
            operation_id.validate()?;
        }
        validate_bytes(&self.payload)?;
        if self.payload_digest != digest_bytes(&self.payload) {
            return Err(ExecutionStoreError::InvalidWorkflowRecord);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowRunSnapshot {
    pub run_id: WorkflowRunId,
    pub workflow_id: WorkflowDefinitionId,
    pub plan_id: WorkflowPlanId,
    pub episode_id: WorkflowEpisodeId,
    pub revision: u64,
    pub projection: RunProjection,
}
