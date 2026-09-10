// SPDX-License-Identifier: MIT

struct OperationEntry {
    state: CoopNativeOperationState,
    digest: String,
    expected_host_generation: Option<u64>,
}

pub struct CoopNativeCoordinator<P> {
    port: P,
    lineage: CoopNativeLineage,
    artifact_lineage: CoopNativeArtifactLineage,
    records: Vec<CoopNativeRecord>,
    operations: BTreeMap<CoopNativeOperationId, OperationEntry>,
    seen_digests: BTreeMap<String, u16>,
}

impl<P: CoopNativePort> CoopNativeCoordinator<P> {
    pub fn new(port: P, lineage: CoopNativeLineage) -> Result<Self, CoopNativeCoordinatorError> {
        let artifact_lineage = CoopNativeArtifactLineage::new(lineage.clone())?;
        Ok(Self {
            port,
            lineage,
            artifact_lineage,
            records: Vec::new(),
            operations: BTreeMap::new(),
            seen_digests: BTreeMap::new(),
        })
    }

    /// Creates a candidate-scoped coordinator. It is explicit that the producer is not admitted.
    pub fn new_candidate(port: P, lineage: CoopNativeLineage) -> Result<Self, CoopNativeCoordinatorError> {
        Self::new(port, lineage)
    }

    #[must_use]
    pub fn artifact_lineage(&self) -> &CoopNativeArtifactLineage {
        &self.artifact_lineage
    }

    #[must_use]
    pub fn records(&self) -> &[CoopNativeRecord] {
        &self.records
    }

    #[must_use]
    pub fn operation_state(&self, operation_id: &CoopNativeOperationId) -> Option<CoopNativeOperationState> {
        self.operations.get(operation_id).map(|entry| entry.state)
    }

    #[must_use]
    pub fn port(&self) -> &P {
        &self.port
    }

    pub fn port_mut(&mut self) -> &mut P {
        &mut self.port
    }

    /// Live consumption remains closed while this candidate has no producer digest parity.
    pub fn consume(&mut self, envelope: CoopNativeEnvelope) -> Result<CoopNativeReceipt, CoopNativeCoordinatorError> {
        if !self.artifact_lineage.is_admitted() {
            return Err(CoopNativeCoordinatorError::Unadmitted);
        }
        self.consume_inner(envelope, true)
    }

    /// Explicit fixture lane for deterministic contract/replay tests before admission.
    pub fn consume_candidate_fixture(
        &mut self,
        envelope: CoopNativeEnvelope,
    ) -> Result<CoopNativeReceipt, CoopNativeCoordinatorError> {
        self.consume_inner(envelope, true)
    }

    pub fn consume_bytes(
        &mut self,
        bytes: &[u8],
    ) -> Result<CoopNativeReceipt, CoopNativeCoordinatorError> {
        self.consume(CoopNativeEnvelope::from_bytes(bytes)?)
    }

    pub fn consume_request_bytes(
        &mut self,
        bytes: &[u8],
    ) -> Result<CoopNativeReceipt, CoopNativeCoordinatorError> {
        self.consume(CoopNativeEnvelope::parse_request(bytes)?)
    }

    pub fn consume_response_bytes(
        &mut self,
        bytes: &[u8],
    ) -> Result<CoopNativeReceipt, CoopNativeCoordinatorError> {
        self.consume(CoopNativeEnvelope::parse_response(bytes)?)
    }

    pub fn consume_candidate_fixture_bytes(
        &mut self,
        bytes: &[u8],
    ) -> Result<CoopNativeReceipt, CoopNativeCoordinatorError> {
        self.consume_candidate_fixture(CoopNativeEnvelope::from_bytes(bytes)?)
    }

    pub fn reconcile(&mut self, envelope: CoopNativeEnvelope) -> Result<CoopNativeReceipt, CoopNativeCoordinatorError> {
        if envelope.kind() != CoopNativeKind::RecoveryResponse {
            return Err(CoopNativeCoordinatorError::WrongReconciliation);
        }
        self.consume(envelope)
    }

    pub fn reconcile_candidate_fixture(
        &mut self,
        envelope: CoopNativeEnvelope,
    ) -> Result<CoopNativeReceipt, CoopNativeCoordinatorError> {
        if envelope.kind() != CoopNativeKind::RecoveryResponse {
            return Err(CoopNativeCoordinatorError::WrongReconciliation);
        }
        self.consume_candidate_fixture(envelope)
    }

    /// Unknown mutation outcomes never trigger a second submission.
    pub fn retry_unknown(
        &mut self,
        operation_id: &CoopNativeOperationId,
    ) -> Result<CoopNativeReceipt, CoopNativeCoordinatorError> {
        if self.operation_state(operation_id) == Some(CoopNativeOperationState::Unknown) {
            Err(CoopNativeCoordinatorError::NoBlindRetry)
        } else {
            Err(CoopNativeCoordinatorError::MissingOperation)
        }
    }

    pub fn artifact_record(&self) -> Result<CoopNativeArtifactRecord, CoopNativeCoordinatorError> {
        let bytes = serde_json::to_vec(&self.records).map_err(|_| CoopNativeCoordinatorError::Serialization)?;
        CoopNativeArtifactRecord::new(self.artifact_lineage.clone(), &bytes).map_err(Into::into)
    }

    pub fn trace_json(&self) -> Result<String, CoopNativeCoordinatorError> {
        serde_json::to_string(&self.records).map_err(|_| CoopNativeCoordinatorError::Serialization)
    }

    fn consume_inner(
        &mut self,
        envelope: CoopNativeEnvelope,
        deliver: bool,
    ) -> Result<CoopNativeReceipt, CoopNativeCoordinatorError> {
        envelope.validate_lineage(&self.lineage)?;
        let canonical = envelope.to_json()?;
        let digest = format!("{:x}", Sha256::digest(canonical.as_bytes()));
        if self.seen_digests.contains_key(&digest) {
            let state = envelope
                .operation_id()
                .and_then(|operation| self.operation_state(operation));
            return self.append_record(&envelope, canonical, digest, CoopNativeEventKind::DuplicateReplay, state);
        }
        let operation = envelope.operation_id().cloned();
        if let Some(operation_id) = &operation
            && let Some(entry) = self.operations.get(operation_id)
            && envelope.kind().is_request()
            && entry.digest != digest
        {
            return Err(CoopNativeCoordinatorError::OperationConflict);
        }
        if let Some(operation_id) = &operation
            && let Some(entry) = self.operations.get(operation_id)
            && let (Some(expected), Some(receipt_generation)) =
                (entry.expected_host_generation, envelope.receipt().map(|receipt| receipt.before_host_generation()))
            && expected != receipt_generation
        {
            return Err(CoopNativeCoordinatorError::OperationConflict);
        }
        let (event, next_state) = self.classify(&envelope, operation.as_ref())?;
        if deliver {
            self.port.consume(&envelope)?;
        }
        let record = self.append_record(&envelope, canonical, digest.clone(), event, next_state)?;
        self.seen_digests.insert(digest.clone(), record.sequence);
        if let Some(operation_id) = operation {
            let state = next_state.ok_or(CoopNativeCoordinatorError::Serialization)?;
            let expected_host_generation = self
                .operations
                .get(&operation_id)
                .and_then(|entry| entry.expected_host_generation)
                .or_else(|| envelope.expected_host_generation());
            self.operations.insert(
                operation_id,
                OperationEntry {
                    state,
                    digest,
                    expected_host_generation,
                },
            );
        }
        Ok(record)
    }

    fn classify(
        &self,
        envelope: &CoopNativeEnvelope,
        operation_id: Option<&CoopNativeOperationId>,
    ) -> Result<(CoopNativeEventKind, Option<CoopNativeOperationState>), CoopNativeCoordinatorError> {
        let current = operation_id.and_then(|operation| self.operation_state(operation));
        Ok(match envelope.kind() {
            CoopNativeKind::Observation => (CoopNativeEventKind::Observation, None),
            CoopNativeKind::LegalCatalogRequest => {
                (CoopNativeEventKind::LegalCatalogRequested, None)
            }
            CoopNativeKind::LegalCatalogResponse => {
                (CoopNativeEventKind::LegalCatalogObserved, None)
            }
            CoopNativeKind::LocalActionRequest => (CoopNativeEventKind::LocalActionRequested, Some(CoopNativeOperationState::Requested)),
            CoopNativeKind::SharedVoteRequest => (CoopNativeEventKind::SharedVoteRequested, Some(CoopNativeOperationState::Requested)),
            CoopNativeKind::RejoinRequest => (CoopNativeEventKind::RejoinRequested, Some(CoopNativeOperationState::Requested)),
            CoopNativeKind::EffectResponse => {
                if current.is_none() { return Err(CoopNativeCoordinatorError::MissingOperation); }
                let status = envelope
                    .effect_response()
                    .ok_or(CoopNativeCoordinatorError::Serialization)?
                    .status();
                if !matches!(
                    (current, status),
                    (
                        Some(CoopNativeOperationState::Requested | CoopNativeOperationState::Accepted),
                        _
                    )
                ) {
                    return Err(CoopNativeCoordinatorError::OperationConflict);
                }
                (status.event(), Some(status.state()))
            }
            CoopNativeKind::RecoveryResponse => self.classify_recovery(envelope.recovery_response().ok_or(CoopNativeCoordinatorError::Serialization)?, current)?,
        })
    }

    fn classify_recovery(
        &self,
        response: &CoopNativeRecoveryResponse,
        current: Option<CoopNativeOperationState>,
    ) -> Result<(CoopNativeEventKind, Option<CoopNativeOperationState>), CoopNativeCoordinatorError> {
        let Some(current) = current else {
            return Err(CoopNativeCoordinatorError::MissingOperation);
        };
        match response.status() {
            None => {
                if current != CoopNativeOperationState::Unknown {
                    return Err(CoopNativeCoordinatorError::ReconciliationMismatch);
                }
                Ok((CoopNativeEventKind::ReconcileRequested, Some(current)))
            }
            Some(super::wire::CoopNativeStatus::Unknown) => {
                if !matches!(
                    current,
                    CoopNativeOperationState::Requested | CoopNativeOperationState::Unknown
                ) {
                    return Err(CoopNativeCoordinatorError::OperationConflict);
                }
                Ok((CoopNativeEventKind::Unknown, Some(CoopNativeOperationState::Unknown)))
            }
            Some(_) => {
                if current != CoopNativeOperationState::Unknown {
                    return Err(CoopNativeCoordinatorError::ReconciliationMismatch);
                }
                Ok((CoopNativeEventKind::Reconciled, Some(CoopNativeOperationState::Reconciled)))
            }
        }
    }

    fn append_record(
        &mut self,
        envelope: &CoopNativeEnvelope,
        envelope_json: String,
        envelope_digest: String,
        event: CoopNativeEventKind,
        state: Option<CoopNativeOperationState>,
    ) -> Result<CoopNativeReceipt, CoopNativeCoordinatorError> {
        if self.records.len() >= COOP_NATIVE_MAX_RECORDS {
            return Err(CoopNativeCoordinatorError::TooManyRecords);
        }
        let sequence = u16::try_from(self.records.len()).map_err(|_| CoopNativeCoordinatorError::TooManyRecords)?;
        let record = CoopNativeRecord {
            sequence,
            event,
            state,
            correlation_id: envelope.header().correlation_id().clone(),
            instance_id: envelope.header().instance_id().clone(),
            session_id: envelope.header().session_id().clone(),
            lease_id: envelope.header().lease_id().clone(),
            lease_epoch: envelope.header().lease_epoch(),
            operation_id: envelope.operation_id().cloned(),
            action_id: envelope.action_id().cloned(),
            envelope_json,
            envelope_digest,
            lineage: self.lineage.clone(),
            artifact_status: self.artifact_lineage.status(),
            admission: self.artifact_lineage.admission(),
        };
        self.records.push(record);
        Ok(CoopNativeReceipt {
            sequence,
            event,
            state,
            operation_id: envelope.operation_id().cloned(),
            artifact_status: self.artifact_lineage.status(),
            admission: self.artifact_lineage.admission(),
        })
    }
}
include!("coop_native_replay.rs");
