// SPDX-License-Identifier: MIT

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeV2Runner;

impl RuntimeV2Runner {
    /// Creates the deterministic fake runner.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Verifies the copied artifact and emits the complete bounded fake trace.
    pub fn run(&self) -> Result<RuntimeV2Report, RuntimeV2Error> {
        verify_runtime_v2_artifact().map_err(RuntimeV2Error::Artifact)?;
        let context = RuntimeV2Context::new(INSTANCE_ID, SESSION_ID, LEASE_ID, LEASE_EPOCH)?;
        let initial_observation = RuntimeV2Observation::new(
            RuntimeV2CombatPhase::PlayerTurn,
            INITIAL_TURN_INDEX,
            true,
            INITIAL_GENERATION,
        )?;
        let mut fake = FakeRuntimeV2::new(context.clone(), initial_observation);
        let mut records = Vec::new();

        let state_request =
            RuntimeV2Message::state_request(&context, "corr-0001", initial_observation.generation)?;
        push_record(
            &mut records,
            RuntimeV2EventKind::Observation,
            RuntimeV2RecordKind::Observation,
            state_request.clone(),
            no_retry_evidence(None, false, 0),
        )?;
        let state_response = fake.state_response(&state_request)?;
        push_record(
            &mut records,
            RuntimeV2EventKind::Observation,
            RuntimeV2RecordKind::Observation,
            state_response,
            no_retry_evidence(None, false, 0),
        )?;

        // The operation identity is allocated before the action request is built or submitted.
        let operation_id = RuntimeV2OperationId::new("op-1")?;
        let action = RuntimeV2Action::end_turn();
        let action_request = RuntimeV2Message::action_request(
            &context,
            "corr-0002",
            initial_observation.generation,
            operation_id.clone(),
            action.clone(),
        )?;
        push_record(
            &mut records,
            RuntimeV2EventKind::Requested,
            RuntimeV2RecordKind::ActionRequested,
            action_request.clone(),
            no_retry_evidence(Some(operation_id.clone()), false, 0),
        )?;

        let accepted = fake.accept_action(&action_request)?;
        if accepted.message.status() != Some(RuntimeV2Status::Accepted)
            || accepted.message.observation() != Some(initial_observation)
        {
            return Err(RuntimeV2Error::Invalid(
                "fake admission did not remain admission-only",
            ));
        }
        push_record(
            &mut records,
            RuntimeV2EventKind::Accepted,
            RuntimeV2RecordKind::ActionAccepted,
            accepted.message,
            no_retry_evidence(Some(operation_id.clone()), false, 0),
        )?;

        match fake.disconnect_after_write(&operation_id, &action_request) {
            Err(RuntimeV2Error::PostWriteDisconnect) => {}
            Ok(()) => {
                return Err(RuntimeV2Error::Invalid(
                    "fake post-write disconnect unexpectedly returned a response",
                ));
            }
            Err(error) => return Err(error),
        }
        let unknown = RuntimeV2Message::action_response(
            &context,
            action_request.correlation_id(),
            initial_observation.generation,
            operation_id.clone(),
            action.clone(),
            None,
            RuntimeV2Status::Unknown,
            Some(UNKNOWN_ERROR.to_owned()),
            None,
        )?;
        push_record(
            &mut records,
            RuntimeV2EventKind::Unknown,
            RuntimeV2RecordKind::Marker,
            unknown,
            no_retry_evidence(Some(operation_id.clone()), true, 1),
        )?;

        let reconcile_request = RuntimeV2Message::reconcile_request(
            &context,
            "corr-0003",
            initial_observation.generation,
            operation_id.clone(),
        )?;
        push_record(
            &mut records,
            RuntimeV2EventKind::Reconciled,
            RuntimeV2RecordKind::Marker,
            reconcile_request.clone(),
            no_retry_evidence(Some(operation_id.clone()), true, 1),
        )?;
        let reconciled = fake.reconcile_action(&reconcile_request)?;
        let settled_generation = initial_observation
            .generation
            .checked_add(1)
            .ok_or(RuntimeV2Error::Invalid("fake generation overflowed"))?;
        let settled_observation =
            reconciled
                .message
                .observation()
                .ok_or(RuntimeV2Error::Invalid(
                    "reconcile result omitted observation",
                ))?;
        let witness = reconciled
            .message
            .effect_witness()
            .ok_or(RuntimeV2Error::Invalid("reconcile result omitted witness"))?;
        if reconciled.message.status() != Some(RuntimeV2Status::Settled)
            || settled_observation.generation != settled_generation
            || witness.kind != WITNESS_KIND
            || witness.generation != settled_generation
        {
            return Err(RuntimeV2Error::Invalid(
                "reconcile did not return a fresh settled witness",
            ));
        }
        push_record(
            &mut records,
            RuntimeV2EventKind::Settled,
            RuntimeV2RecordKind::ActionCompleted,
            reconciled.message,
            no_retry_evidence(Some(operation_id.clone()), true, 1),
        )?;

        let fresh_state_request =
            RuntimeV2Message::state_request(&context, "corr-0004", settled_generation)?;
        push_record(
            &mut records,
            RuntimeV2EventKind::Observation,
            RuntimeV2RecordKind::Observation,
            fresh_state_request.clone(),
            no_retry_evidence(Some(operation_id.clone()), true, 1),
        )?;
        let fresh_state = fake.state_response(&fresh_state_request)?;
        if fresh_state.generation() != settled_generation
            || fresh_state.observation().map(|value| value.generation) != Some(settled_generation)
        {
            return Err(RuntimeV2Error::Invalid(
                "fresh state did not advance exactly one generation",
            ));
        }
        push_record(
            &mut records,
            RuntimeV2EventKind::Observation,
            RuntimeV2RecordKind::Observation,
            fresh_state,
            no_retry_evidence(Some(operation_id.clone()), true, 1),
        )?;

        let duplicate = fake.accept_action(&action_request)?;
        if !duplicate.replayed
            || duplicate.message.status() != Some(RuntimeV2Status::Settled)
            || fake.mutation_count() != 1
        {
            return Err(RuntimeV2Error::Invalid(
                "duplicate action did not replay without a second mutation",
            ));
        }
        push_record(
            &mut records,
            RuntimeV2EventKind::DuplicateReplay,
            RuntimeV2RecordKind::ActionRequested,
            action_request,
            no_retry_evidence(Some(operation_id.clone()), true, 1),
        )?;
        push_record(
            &mut records,
            RuntimeV2EventKind::DuplicateReplay,
            RuntimeV2RecordKind::ActionCompleted,
            duplicate.message,
            no_retry_evidence(Some(operation_id.clone()), true, 1),
        )?;

        let stale_context = RuntimeV2Context::new(INSTANCE_ID, SESSION_ID, LEASE_ID, 0)?;
        let stale_operation_id = RuntimeV2OperationId::new("op-stale-epoch")?;
        let stale_request = RuntimeV2Message::action_request(
            &stale_context,
            "corr-0005",
            settled_generation,
            stale_operation_id.clone(),
            action.clone(),
        )?;
        push_record(
            &mut records,
            RuntimeV2EventKind::Requested,
            RuntimeV2RecordKind::ActionRequested,
            stale_request.clone(),
            no_retry_evidence(Some(stale_operation_id.clone()), false, 0),
        )?;
        let stale_result = fake.accept_action(&stale_request)?;
        if stale_result.message.status() != Some(RuntimeV2Status::Rejected)
            || stale_result.message.error_code() != Some(STALE_EPOCH_ERROR)
            || fake.mutation_count() != 1
        {
            return Err(RuntimeV2Error::Invalid(
                "stale epoch was not rejected before mutation",
            ));
        }
        push_record(
            &mut records,
            RuntimeV2EventKind::Rejected,
            RuntimeV2RecordKind::Marker,
            stale_result.message,
            no_retry_evidence(Some(stale_operation_id), false, 0),
        )?;

        let lineage = RuntimeV2ArtifactLineage::new(TRAJECTORY_ID)?;
        let trajectory =
            RuntimeV2Trajectory::new(TRAJECTORY_ID, &context, lineage.clone(), records)?;
        let trajectory_bytes =
            serde_json::to_vec(&trajectory).map_err(|_| RuntimeV2Error::Encode)?;
        let content_digest = format!("{:x}", Sha256::digest(&trajectory_bytes));
        let artifact = RuntimeV2ArtifactRecord::new(
            TRACE_ARTIFACT_ID,
            content_digest,
            trajectory_bytes.len() as u64,
            lineage,
        )?;
        let evidence = RuntimeV2Evidence {
            operation_id,
            initial_generation: INITIAL_GENERATION,
            settled_generation,
            mutation_count: fake.mutation_count(),
            duplicate_replay_without_second_application: duplicate.replayed
                && fake.mutation_count() == 1,
            stale_epoch_rejected: true,
            no_blind_retry_after_disconnect: true,
            live_host_settlement: "unverified",
            provider_model_lane: "unverified",
        };
        let document = RuntimeV2TraceDocument {
            artifact: &artifact,
            trajectory: &trajectory,
            evidence: &evidence,
        };
        let trace_bytes = serde_json::to_string(&document).map_err(|_| RuntimeV2Error::Encode)?;
        Ok(RuntimeV2Report {
            trace_bytes: format!("{trace_bytes}\n"),
            trajectory,
            artifact,
            evidence,
        })
    }
}

/// Runs the deterministic one-instance Runtime-v2 fake trace.
pub fn run_runtime_v2_fake_trace() -> Result<RuntimeV2Report, RuntimeV2Error> {
    RuntimeV2Runner::new().run()
}
