// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, BTreeSet};

use crate::artifact::{
    ArtifactDraft, ArtifactMetadata, ArtifactMetadataInput, ArtifactPort,
    ArtifactPublicationRequest, ArtifactReceipt,
};
use crate::error::{CloseReport, HarnessError};
use crate::identity::{
    ArtifactId, EpisodeId, IdAllocator, IdempotencyKey, ModelExecutionId, RequestId, RunId,
    TraceId, TrajectoryId,
};
use crate::provider::{ModelRequest, ModelResponse, ModelResult, ProviderPort, RetryPolicy};
use crate::records::{AppendOutcome, Correlation, Record, RecordKind, RecordPayload, RecordPort};
use crate::replay::{ReplayPort, ReplayReport, ReplayRequest};
use crate::routing::{InstanceRouter, RouteBinding, RouteRequest};

mod lifecycle;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpisodeHandle {
    run_id: RunId,
    episode_id: EpisodeId,
    trajectory_id: TrajectoryId,
    binding: RouteBinding,
    correlation: Correlation,
}

impl EpisodeHandle {
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
    pub const fn instance_id(&self) -> crate::identity::InstanceId {
        self.binding.instance_id()
    }

    #[must_use]
    pub const fn binding(&self) -> &RouteBinding {
        &self.binding
    }

    #[must_use]
    pub const fn correlation(&self) -> &Correlation {
        &self.correlation
    }
}

struct ActiveEpisode {
    handle: EpisodeHandle,
    next_sequence: u64,
    records: BTreeMap<IdempotencyKey, Record>,
}

pub struct HarnessParts<R, P, S, A, X> {
    pub router: R,
    pub provider: P,
    pub storage: S,
    pub artifacts: A,
    pub replay: X,
}

pub struct Harness<R, P, S, A, X> {
    ids: IdAllocator,
    runs: BTreeSet<RunId>,
    episodes: BTreeMap<EpisodeId, ActiveEpisode>,
    router: R,
    provider: P,
    storage: S,
    artifacts: A,
    replay: X,
    closed_outcome: Option<Result<CloseReport, HarnessError>>,
}

impl<R, P, S, A, X> Harness<R, P, S, A, X>
where
    R: InstanceRouter,
    P: ProviderPort,
    S: RecordPort,
    A: ArtifactPort,
    X: ReplayPort,
{
    #[must_use]
    pub fn new(router: R, provider: P, storage: S, artifacts: A, replay: X) -> Self {
        Self::with_seed(1, router, provider, storage, artifacts, replay)
    }

    #[must_use]
    pub fn with_seed(
        seed: u64,
        router: R,
        provider: P,
        storage: S,
        artifacts: A,
        replay: X,
    ) -> Self {
        Self {
            ids: IdAllocator::new(seed),
            runs: BTreeSet::new(),
            episodes: BTreeMap::new(),
            router,
            provider,
            storage,
            artifacts,
            replay,
            closed_outcome: None,
        }
    }

    pub fn start_run(&mut self) -> Result<RunId, HarnessError> {
        self.ensure_open()?;
        let run_id = self.ids.allocate()?;
        self.runs.insert(run_id);
        Ok(run_id)
    }

    pub fn start_episode(
        &mut self,
        run_id: RunId,
        preferred_instance: Option<crate::identity::InstanceId>,
    ) -> Result<EpisodeHandle, HarnessError> {
        self.ensure_open()?;
        if !self.runs.contains(&run_id) {
            return Err(HarnessError::Invalid("run is not active".to_owned()));
        }

        let episode_id = self.ids.allocate()?;
        let trajectory_id = self.ids.allocate()?;
        let trace_id = self.ids.allocate::<TraceId>()?;
        let request = RouteRequest::new(run_id, episode_id, preferred_instance);
        let binding = self.router.bind(&request).map_err(HarnessError::Routing)?;
        if binding.run_id() != run_id || binding.episode_id() != episode_id {
            return Err(HarnessError::Invalid(
                "router returned a binding for another run or episode".to_owned(),
            ));
        }
        let correlation = Correlation::for_episode(
            run_id,
            episode_id,
            trajectory_id,
            binding.instance_id(),
            trace_id,
        );
        let handle = EpisodeHandle {
            run_id,
            episode_id,
            trajectory_id,
            binding,
            correlation,
        };
        self.episodes.insert(
            episode_id,
            ActiveEpisode {
                handle: handle.clone(),
                next_sequence: 0,
                records: BTreeMap::new(),
            },
        );
        Ok(handle)
    }

    pub fn record(
        &mut self,
        episode: &EpisodeHandle,
        kind: RecordKind,
        payload: RecordPayload,
        idempotency_key: IdempotencyKey,
    ) -> Result<AppendOutcome, HarnessError> {
        self.ensure_open()?;
        let (trajectory_id, sequence, correlation) = {
            let active = self.active_episode(episode)?;
            if let Some(record) = active.records.get(&idempotency_key) {
                return Ok(AppendOutcome::duplicate_record(record.clone()));
            }
            if active.next_sequence == u64::MAX {
                return Err(HarnessError::Invalid(
                    "trajectory sequence is exhausted".to_owned(),
                ));
            }
            (
                active.handle.trajectory_id,
                active.next_sequence,
                active.handle.correlation.clone(),
            )
        };
        let record_id = self.ids.allocate::<crate::identity::RecordId>()?;
        let record = Record::new(
            record_id,
            trajectory_id,
            sequence,
            correlation,
            kind,
            idempotency_key.clone(),
            payload,
        );
        let outcome = self.storage.append(record).map_err(HarnessError::Storage)?;
        let active = self.active_episode_mut(episode)?;
        if outcome.was_inserted() {
            active.next_sequence = sequence + 1;
        }
        active
            .records
            .insert(idempotency_key, outcome.record().clone());
        Ok(outcome)
    }

    pub fn execute_model(
        &mut self,
        episode: &EpisodeHandle,
        prompt: crate::provider::Prompt,
        retry_policy: RetryPolicy,
    ) -> Result<ModelResult, HarnessError> {
        self.ensure_open()?;
        let correlation = self.active_episode(episode)?.handle.correlation.clone();
        let request_id = self.ids.allocate::<RequestId>()?;
        let execution_id = self.ids.allocate::<ModelExecutionId>()?;
        let idempotency_key =
            IdempotencyKey::new(format!("model-execution-{}", execution_id.get()))?;
        let correlation = correlation
            .with_request(request_id)
            .with_model_execution(execution_id);
        let request = ModelRequest::new(execution_id, correlation, prompt, idempotency_key);
        let mut attempts = 0;
        loop {
            attempts += 1;
            match self.provider.execute(&request) {
                Ok(response) => {
                    validate_response(&request, &response)?;
                    return Ok(crate::provider::model_result(request, response, attempts));
                }
                Err(error) if error.is_retryable() && attempts < retry_policy.max_attempts() => {}
                Err(error) => return Err(HarnessError::Provider(error)),
            }
        }
    }

    pub fn replay_episode(
        &mut self,
        episode: &EpisodeHandle,
    ) -> Result<ReplayReport, HarnessError> {
        self.ensure_open()?;
        let trajectory_id = self.active_episode(episode)?.handle.trajectory_id;
        let records = self
            .storage
            .read(trajectory_id)
            .map_err(HarnessError::Storage)?;
        self.replay
            .replay(ReplayRequest::new(trajectory_id, records))
            .map_err(HarnessError::Replay)
    }

    pub fn publish_artifact(
        &mut self,
        run_id: RunId,
        request: ArtifactPublicationRequest,
    ) -> Result<ArtifactReceipt, HarnessError> {
        self.ensure_open()?;
        if !self.runs.contains(&run_id) {
            return Err(HarnessError::Invalid("run is not active".to_owned()));
        }
        let artifact_id = self.ids.allocate::<ArtifactId>()?;
        let metadata = ArtifactMetadata::from_input(
            ArtifactMetadataInput {
                artifact_id,
                owner_run: run_id,
                kind: request.kind(),
                schema_version: request.schema_version(),
                content_digest: request.content_digest().clone(),
                producer: request.producer().to_owned(),
                lineage: request.lineage().clone(),
            },
            request.bytes().len() as u64,
        )
        .map_err(HarnessError::Artifact)?;
        let draft = ArtifactDraft::new(metadata, request.bytes().to_vec())
            .map_err(HarnessError::Artifact)?;
        self.artifacts
            .publish(draft)
            .map_err(HarnessError::Artifact)
    }
}

fn validate_response(request: &ModelRequest, response: &ModelResponse) -> Result<(), HarnessError> {
    if response.execution_id() != request.execution_id()
        || response.correlation() != request.correlation()
    {
        return Err(HarnessError::Provider(crate::error::ProviderError::new(
            "provider_correlation_mismatch",
            "provider response correlation does not match the request",
            false,
        )));
    }
    Ok(())
}
