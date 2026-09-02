// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::error::Error;

use sts2_harness::{
    AppendOutcome, ArtifactKind, ArtifactLineage, ArtifactMetadata, ArtifactPort,
    ArtifactPublicationRequest, ArtifactReceipt, DeterministicReplay, EpisodeHandle,
    GatewaySessionId, Harness, IdempotencyKey, InstanceId, InstanceRouter, ModelOutput,
    ModelRequest, ModelResponse, PortError, Prompt, ProviderError, ProviderPort, Record,
    RecordKind, RecordPayload, RecordPort, ReplayPort, ReplayReport, ReplayRequest, RetryPolicy,
    RouteBinding, RouteRequest, RouteToken, RunId, SchemaVersion, TrajectoryId,
};

struct FakeRouter {
    next_instance: u64,
    next_session: u64,
    bindings: Vec<RouteBinding>,
    unbindings: Vec<RouteBinding>,
    close_calls: usize,
    closed: bool,
}

impl FakeRouter {
    fn new() -> Self {
        Self {
            next_instance: 1,
            next_session: 1,
            bindings: Vec::new(),
            unbindings: Vec::new(),
            close_calls: 0,
            closed: false,
        }
    }
}

impl InstanceRouter for FakeRouter {
    fn bind(&mut self, request: &RouteRequest) -> Result<RouteBinding, PortError> {
        if self.closed {
            return Err(PortError::new("router_closed", "router is closed", false));
        }
        let instance_id = match request.preferred_instance() {
            Some(instance_id) => instance_id,
            None => {
                let instance_id = InstanceId::new(self.next_instance).ok_or_else(|| {
                    PortError::new("invalid_instance", "instance id is zero", false)
                })?;
                self.next_instance += 1;
                instance_id
            }
        };
        let session_id = GatewaySessionId::new(self.next_session)
            .ok_or_else(|| PortError::new("invalid_session", "session id is zero", false))?;
        self.next_session += 1;
        let route_token = RouteToken::new(format!("route-{}", self.bindings.len() + 1))?;
        let binding = RouteBinding::new(
            request.run_id(),
            request.episode_id(),
            instance_id,
            session_id,
            route_token,
        );
        self.bindings.push(binding.clone());
        Ok(binding)
    }

    fn unbind(&mut self, binding: &RouteBinding) -> Result<(), PortError> {
        if self.closed {
            return Err(PortError::new("router_closed", "router is closed", false));
        }
        self.unbindings.push(binding.clone());
        Ok(())
    }

    fn close(&mut self) -> Result<(), PortError> {
        self.close_calls += 1;
        self.closed = true;
        Ok(())
    }
}

struct FakeProvider {
    failures_left: u8,
    requests: Vec<ModelRequest>,
    responses: BTreeMap<IdempotencyKey, ModelResponse>,
    close_calls: usize,
    closed: bool,
}

impl FakeProvider {
    fn with_failures(failures_left: u8) -> Self {
        Self {
            failures_left,
            requests: Vec::new(),
            responses: BTreeMap::new(),
            close_calls: 0,
            closed: false,
        }
    }
}

impl ProviderPort for FakeProvider {
    fn execute(&mut self, request: &ModelRequest) -> Result<ModelResponse, ProviderError> {
        if self.closed {
            return Err(ProviderError::new(
                "provider_closed",
                "provider is closed",
                false,
            ));
        }
        self.requests.push(request.clone());
        if self.failures_left > 0 {
            self.failures_left -= 1;
            return Err(ProviderError::new(
                "transient_provider_failure",
                "deterministic fake failure",
                true,
            ));
        }
        if let Some(response) = self.responses.get(request.idempotency_key()) {
            return Ok(response.clone());
        }
        let output =
            ModelOutput::new(format!("fake:{}", request.prompt().as_str())).map_err(|error| {
                ProviderError::new(error.code(), error.message(), error.retryable())
            })?;
        let response = ModelResponse::new(
            request.execution_id(),
            request.correlation().clone(),
            output,
        )?;
        self.responses
            .insert(request.idempotency_key().clone(), response.clone());
        Ok(response)
    }

    fn close(&mut self) -> Result<(), PortError> {
        self.close_calls += 1;
        self.closed = true;
        Ok(())
    }
}

struct FakeStorage {
    records: BTreeMap<TrajectoryId, Vec<Record>>,
    by_key: BTreeMap<(TrajectoryId, IdempotencyKey), Record>,
    append_calls: usize,
    close_calls: usize,
    closed: bool,
}

impl FakeStorage {
    fn new() -> Self {
        Self {
            records: BTreeMap::new(),
            by_key: BTreeMap::new(),
            append_calls: 0,
            close_calls: 0,
            closed: false,
        }
    }
}

impl RecordPort for FakeStorage {
    fn append(&mut self, record: Record) -> Result<AppendOutcome, PortError> {
        if self.closed {
            return Err(PortError::new("storage_closed", "storage is closed", false));
        }
        self.append_calls += 1;
        let key = record.idempotency_key().clone();
        let scope = (record.trajectory_id(), key.clone());
        if let Some(existing) = self.by_key.get(&scope) {
            return Ok(AppendOutcome::duplicate_record(existing.clone()));
        }
        self.records
            .entry(record.trajectory_id())
            .or_default()
            .push(record.clone());
        self.by_key.insert(scope, record.clone());
        Ok(AppendOutcome::inserted_record(record))
    }

    fn read(&self, trajectory_id: TrajectoryId) -> Result<Vec<Record>, PortError> {
        match self.records.get(&trajectory_id) {
            Some(records) => Ok(records.clone()),
            None => Ok(Vec::new()),
        }
    }

    fn close(&mut self) -> Result<(), PortError> {
        self.close_calls += 1;
        self.closed = true;
        Ok(())
    }
}

struct FakeArtifacts {
    published: Vec<(ArtifactMetadata, usize)>,
    close_calls: usize,
    closed: bool,
}

impl FakeArtifacts {
    fn new() -> Self {
        Self {
            published: Vec::new(),
            close_calls: 0,
            closed: false,
        }
    }
}

impl ArtifactPort for FakeArtifacts {
    fn publish(
        &mut self,
        draft: sts2_harness::ArtifactDraft,
    ) -> Result<ArtifactReceipt, PortError> {
        if self.closed {
            return Err(PortError::new(
                "artifact_closed",
                "artifact port is closed",
                false,
            ));
        }
        let metadata = draft.metadata().clone();
        self.published.push((metadata.clone(), draft.bytes().len()));
        Ok(ArtifactReceipt::new(metadata, true))
    }

    fn close(&mut self) -> Result<(), PortError> {
        self.close_calls += 1;
        self.closed = true;
        Ok(())
    }
}

struct FakeReplay {
    requests: Vec<ReplayRequest>,
    close_calls: usize,
    closed: bool,
}

impl FakeReplay {
    fn new() -> Self {
        Self {
            requests: Vec::new(),
            close_calls: 0,
            closed: false,
        }
    }
}

impl ReplayPort for FakeReplay {
    fn replay(&mut self, request: ReplayRequest) -> Result<ReplayReport, PortError> {
        if self.closed {
            return Err(PortError::new("replay_closed", "replay is closed", false));
        }
        let report = DeterministicReplay::evaluate(&request);
        self.requests.push(request);
        Ok(report)
    }

    fn close(&mut self) -> Result<(), PortError> {
        self.close_calls += 1;
        self.closed = true;
        Ok(())
    }
}

type TestHarness = Harness<FakeRouter, FakeProvider, FakeStorage, FakeArtifacts, FakeReplay>;

fn harness(provider: FakeProvider) -> TestHarness {
    Harness::new(
        FakeRouter::new(),
        provider,
        FakeStorage::new(),
        FakeArtifacts::new(),
        FakeReplay::new(),
    )
}

fn run_and_episode(harness: &mut TestHarness) -> Result<(RunId, EpisodeHandle), Box<dyn Error>> {
    let run_id = harness.start_run()?;
    let episode = harness.start_episode(run_id, None)?;
    Ok((run_id, episode))
}

#[test]
fn model_retry_reuses_correlation_and_idempotency() -> Result<(), Box<dyn Error>> {
    let mut harness = harness(FakeProvider::with_failures(1));
    let (_, episode) = run_and_episode(&mut harness)?;
    let result = harness.execute_model(&episode, Prompt::new("choose")?, RetryPolicy::new(3)?)?;

    assert_eq!(result.attempts(), 2);
    assert_eq!(
        result.request().correlation(),
        result.response().correlation()
    );
    assert_eq!(
        result.request().execution_id(),
        result.response().execution_id()
    );
    let parts = harness.into_parts();
    assert_eq!(parts.provider.requests.len(), 2);
    assert_eq!(
        parts.provider.requests[0].correlation(),
        parts.provider.requests[1].correlation()
    );
    assert_eq!(
        parts.provider.requests[0].idempotency_key(),
        parts.provider.requests[1].idempotency_key()
    );
    assert_eq!(
        episode.instance_id(),
        result.request().correlation().instance_id()
    );
    Ok(())
}

#[test]
fn records_are_idempotent_and_replayable() -> Result<(), Box<dyn Error>> {
    let mut harness = harness(FakeProvider::with_failures(0));
    let (_, episode) = run_and_episode(&mut harness)?;
    let first = harness.record(
        &episode,
        RecordKind::Observation,
        RecordPayload::new(b"observation".to_vec())?,
        IdempotencyKey::new("observation-1")?,
    )?;
    let duplicate = harness.record(
        &episode,
        RecordKind::Observation,
        RecordPayload::new(b"different-payload-is-ignored".to_vec())?,
        IdempotencyKey::new("observation-1")?,
    )?;
    let second = harness.record(
        &episode,
        RecordKind::Marker,
        RecordPayload::new(b"marker".to_vec())?,
        IdempotencyKey::new("marker-1")?,
    )?;

    assert!(first.inserted());
    assert!(!duplicate.inserted());
    assert_eq!(first.record(), duplicate.record());
    assert_eq!(second.record().sequence(), 1);
    let report = harness.replay_episode(&episode)?;
    assert_eq!(report.records_replayed(), 2);
    assert_eq!(report.last_sequence(), Some(1));
    assert!(report.divergence().is_none());

    let parts = harness.into_parts();
    assert_eq!(parts.storage.append_calls, 2);
    assert_eq!(parts.storage.records[&episode.trajectory_id()].len(), 2);
    assert_eq!(parts.replay.requests.len(), 1);
    Ok(())
}

#[test]
fn artifact_metadata_is_lineage_bound() -> Result<(), Box<dyn Error>> {
    let mut harness = harness(FakeProvider::with_failures(0));
    let (run_id, episode) = run_and_episode(&mut harness)?;
    let schema_version = SchemaVersion::new(1).ok_or("schema version cannot be zero")?;
    let digest = sts2_harness::Digest::new("a".repeat(64))?;
    let lineage = ArtifactLineage::new(run_id, Some(episode.trajectory_id()), Vec::new())?;
    let request = ArtifactPublicationRequest::new(
        ArtifactKind::Trajectory,
        schema_version,
        digest,
        "deterministic-test",
        b"trajectory-bytes".to_vec(),
        lineage,
    );
    let receipt = harness.publish_artifact(run_id, request)?;

    assert!(receipt.published());
    assert_eq!(receipt.metadata().owner_run(), run_id);
    assert_eq!(receipt.metadata().lineage().source_run(), run_id);
    assert_eq!(
        receipt.metadata().lineage().source_trajectory(),
        Some(episode.trajectory_id())
    );
    assert_eq!(receipt.metadata().schema_version(), schema_version);
    assert_eq!(receipt.metadata().kind(), ArtifactKind::Trajectory);
    assert_eq!(receipt.metadata().producer(), "deterministic-test");
    assert_eq!(receipt.metadata().byte_length(), 16);

    let parts = harness.into_parts();
    assert_eq!(parts.artifacts.published.len(), 1);
    assert_eq!(parts.artifacts.published[0].1, 16);
    Ok(())
}

#[test]
fn close_unbinds_and_closes_every_port_once() -> Result<(), Box<dyn Error>> {
    let mut harness = harness(FakeProvider::with_failures(0));
    let (_, episode) = run_and_episode(&mut harness)?;
    let report = harness.close()?;
    assert!(report.is_clean());
    assert_eq!(report.unbound_episodes, 1);
    assert_eq!(report.closed_components.len(), 5);
    assert_eq!(harness.close()?, report);

    let parts = harness.into_parts();
    assert_eq!(parts.router.unbindings.len(), 1);
    assert_eq!(parts.router.close_calls, 1);
    assert_eq!(parts.provider.close_calls, 1);
    assert_eq!(parts.storage.close_calls, 1);
    assert_eq!(parts.artifacts.close_calls, 1);
    assert_eq!(parts.replay.close_calls, 1);
    assert_eq!(parts.router.unbindings[0], episode.binding().clone());
    Ok(())
}
