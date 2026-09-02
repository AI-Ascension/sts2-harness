// SPDX-License-Identifier: MIT

use crate::error::PortError;
use crate::identity::{
    ActionId, EpisodeId, IdempotencyKey, InstanceId, ModelExecutionId, RecordId, RequestId, RunId,
    TraceId, TrajectoryId,
};

const MAX_RECORD_PAYLOAD_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Correlation {
    run_id: RunId,
    episode_id: EpisodeId,
    trajectory_id: TrajectoryId,
    instance_id: InstanceId,
    trace_id: TraceId,
    request_id: Option<RequestId>,
    action_id: Option<ActionId>,
    model_execution_id: Option<ModelExecutionId>,
}

impl Correlation {
    #[must_use]
    pub const fn for_episode(
        run_id: RunId,
        episode_id: EpisodeId,
        trajectory_id: TrajectoryId,
        instance_id: InstanceId,
        trace_id: TraceId,
    ) -> Self {
        Self {
            run_id,
            episode_id,
            trajectory_id,
            instance_id,
            trace_id,
            request_id: None,
            action_id: None,
            model_execution_id: None,
        }
    }

    #[must_use]
    pub const fn with_request(mut self, request_id: RequestId) -> Self {
        self.request_id = Some(request_id);
        self
    }

    #[must_use]
    pub const fn with_action(mut self, action_id: ActionId) -> Self {
        self.action_id = Some(action_id);
        self
    }

    #[must_use]
    pub const fn with_model_execution(mut self, execution_id: ModelExecutionId) -> Self {
        self.model_execution_id = Some(execution_id);
        self
    }

    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    #[must_use]
    pub const fn episode_id(&self) -> EpisodeId {
        self.episode_id
    }

    #[must_use]
    pub const fn trajectory_id(&self) -> TrajectoryId {
        self.trajectory_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    #[must_use]
    pub const fn trace_id(&self) -> TraceId {
        self.trace_id
    }

    #[must_use]
    pub const fn request_id(&self) -> Option<RequestId> {
        self.request_id
    }

    #[must_use]
    pub const fn action_id(&self) -> Option<ActionId> {
        self.action_id
    }

    #[must_use]
    pub const fn model_execution_id(&self) -> Option<ModelExecutionId> {
        self.model_execution_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordKind {
    Observation,
    ActionRequested,
    ActionAccepted,
    ActionCompleted,
    ModelRequested,
    ModelCompleted,
    Marker,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordPayload(Vec<u8>);

impl RecordPayload {
    pub fn new(value: Vec<u8>) -> Result<Self, PortError> {
        if value.len() > MAX_RECORD_PAYLOAD_BYTES {
            return Err(PortError::new(
                "record_payload_too_large",
                "record payload exceeds its bound",
                false,
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Record {
    record_id: RecordId,
    trajectory_id: TrajectoryId,
    sequence: u64,
    correlation: Correlation,
    kind: RecordKind,
    idempotency_key: IdempotencyKey,
    payload: RecordPayload,
}

impl Record {
    #[must_use]
    pub const fn new(
        record_id: RecordId,
        trajectory_id: TrajectoryId,
        sequence: u64,
        correlation: Correlation,
        kind: RecordKind,
        idempotency_key: IdempotencyKey,
        payload: RecordPayload,
    ) -> Self {
        Self {
            record_id,
            trajectory_id,
            sequence,
            correlation,
            kind,
            idempotency_key,
            payload,
        }
    }

    #[must_use]
    pub const fn record_id(&self) -> RecordId {
        self.record_id
    }

    #[must_use]
    pub const fn trajectory_id(&self) -> TrajectoryId {
        self.trajectory_id
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub const fn correlation(&self) -> &Correlation {
        &self.correlation
    }

    #[must_use]
    pub const fn kind(&self) -> RecordKind {
        self.kind
    }

    #[must_use]
    pub fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }

    #[must_use]
    pub fn payload(&self) -> &RecordPayload {
        &self.payload
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppendOutcome {
    record: Record,
    was_inserted: bool,
}

impl AppendOutcome {
    #[must_use]
    pub fn record(&self) -> &Record {
        &self.record
    }

    #[must_use]
    pub const fn was_inserted(&self) -> bool {
        self.was_inserted
    }

    #[must_use]
    pub fn inserted_record(record: Record) -> Self {
        Self {
            record,
            was_inserted: true,
        }
    }

    #[must_use]
    pub fn duplicate_record(record: Record) -> Self {
        Self {
            record,
            was_inserted: false,
        }
    }
}

pub trait RecordPort {
    fn append(&mut self, record: Record) -> Result<AppendOutcome, PortError>;

    fn read(&self, trajectory_id: TrajectoryId) -> Result<Vec<Record>, PortError>;

    fn close(&mut self) -> Result<(), PortError>;
}
