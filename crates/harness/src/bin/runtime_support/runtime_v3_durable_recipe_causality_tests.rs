// SPDX-License-Identifier: MIT

use super::super::super::super::super::runtime_v3_telemetry::TelemetryHandle;
use super::super::super::super::recording::DecisionRecorder;
use super::*;
use sts2_harness::{DecisionSource, PolicyError};

struct ProbeSource {
    calls: usize,
    decision: Decision,
    durable: DurableHandle,
    context_status_at_call: Option<RecipeInvocationStatus>,
    reservation_present_at_call: bool,
}

impl DecisionSource for ProbeSource {
    fn decide(&mut self, _input: &DecisionInput) -> Result<Decision, PolicyError> {
        self.calls += 1;
        self.context_status_at_call = super::invocation_status(&self.durable).ok();
        self.reservation_present_at_call = match self.durable.store.try_borrow() {
            Ok(store) => store
                .provider_reservation("provider-reservation-model-execution-1")
                .is_ok(),
            Err(_) => false,
        };
        Ok(self.decision.clone())
    }
}

fn action_decision() -> Decision {
    Decision::Action {
        action_id: String::from("move-1"),
        rationale: String::from("synthetic map receipt decision"),
        confidence: Some(90),
    }
}

#[test]
fn decision_recorder_reuses_completed_map_decision_for_same_accepted_input()
-> Result<(), Box<dyn std::error::Error>> {
    let durable = durable_fixture()?;
    let actions = map_actions()?;
    let reads = Cell::new(0_u8);
    let response = map_response("map-1");
    collect_with_intent_check(&durable, &actions, response.clone(), &reads)?;
    let input = mapped_input(response)?;
    let decision = action_decision();
    let mut source = ProbeSource {
        calls: 0,
        decision: decision.clone(),
        durable: durable.clone(),
        context_status_at_call: None,
        reservation_present_at_call: false,
    };
    let mut recorder =
        DecisionRecorder::with_durable(&mut source, TelemetryHandle::disabled(), durable.clone());

    let first = recorder.decide(&input)?;
    assert_eq!(first, decision);
    let reservation_before = durable
        .store
        .try_borrow()?
        .provider_reservation("provider-reservation-model-execution-1")?;
    let stored_before = durable.store.try_borrow()?.decision("model-execution-1")?;
    assert!(stored_before.completed);

    let second = recorder.decide(&input)?;
    assert_eq!(second, first);
    drop(recorder);

    assert_eq!(source.calls, 1);
    assert_eq!(
        source.context_status_at_call,
        Some(RecipeInvocationStatus::ContextValidated)
    );
    assert!(source.reservation_present_at_call);
    assert_eq!(reads.get(), 1);
    assert_eq!(
        invocation_status(&durable)?,
        RecipeInvocationStatus::ContextValidated
    );
    let reservation_after = durable
        .store
        .try_borrow()?
        .provider_reservation("provider-reservation-model-execution-1")?;
    let stored_after = durable.store.try_borrow()?.decision("model-execution-1")?;
    assert_eq!(reservation_after, reservation_before);
    assert_eq!(stored_after, stored_before);
    Ok(())
}

#[test]
fn file_backed_intent_reopen_refuses_read_and_provider_admission()
-> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join(format!(
        "sts2-runtime-map-causality-intent-{}-{}.sqlite3",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let (durable, lineage, fingerprint) = durable_fixture_with_store(ExecutionStore::open(
            ExecutionStoreConfig::new(path.clone()),
        )?)?;
        let binding = invocation_binding(&durable)?;
        durable.begin_map_invocation(binding)?;
        assert_eq!(
            invocation_status(&durable)?,
            RecipeInvocationStatus::IntentRecorded
        );
        durable.close()?;
        drop(durable);

        let store = ExecutionStore::open(ExecutionStoreConfig::new(path.clone()))?;
        assert!(matches!(
            store.resume_episode(&lineage.episode_id, &fingerprint)?,
            sts2_harness::ResumeState::Ready { .. }
        ));
        let reopened = DurableHandle::from_store_for_lifecycle_test(
            store,
            lineage,
            fingerprint,
            String::from("model-revision-1"),
            "e".repeat(64),
        )?;
        let actions = map_actions()?;
        let response = map_response("map-1");
        // This accepted input is synthesized independently by the test owner parser, not read from
        // the receipt row. The reopened durable collector must still refuse its read closure.
        let input = mapped_input(response.clone())?;
        let mut source = ProbeSource {
            calls: 0,
            decision: action_decision(),
            durable: reopened.clone(),
            context_status_at_call: None,
            reservation_present_at_call: false,
        };
        let mut recorder = DecisionRecorder::with_durable(
            &mut source,
            TelemetryHandle::disabled(),
            reopened.clone(),
        );
        assert_eq!(recorder.decide(&input), Err(PolicyError::InputBlocked));
        drop(recorder);
        assert_eq!(source.calls, 0);
        assert!(
            reopened
                .store
                .try_borrow()?
                .provider_reservation("provider-reservation-model-execution-1")
                .is_err()
        );

        let reads = Cell::new(0_u8);
        let retry = collect_map_snapshot(
            &reopened,
            "state-1",
            1,
            ModelExecutionId::new(1).ok_or("execution identity must be nonzero")?,
            &actions,
            || {
                reads.set(reads.get() + 1);
                Ok(response)
            },
        );
        assert!(retry.is_err());
        assert_eq!(reads.get(), 0);
        assert_eq!(
            invocation_status(&reopened)?,
            RecipeInvocationStatus::IntentRecorded
        );
        drop(source);
        reopened.close()?;
        Ok::<(), Box<dyn std::error::Error>>(())
    })();
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    result
}

#[test]
fn file_backed_context_validated_reopen_requires_matching_caller_input_and_fingerprint()
-> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join(format!(
        "sts2-runtime-map-causality-context-{}-{}.sqlite3",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let (durable, lineage, fingerprint) = durable_fixture_with_store(ExecutionStore::open(
            ExecutionStoreConfig::new(path.clone()),
        )?)?;
        let actions = map_actions()?;
        let reads = Cell::new(0_u8);
        let response = map_response("map-1");
        collect_with_intent_check(&durable, &actions, response.clone(), &reads)?;
        // This owner-accepted DecisionInput is caller-held synthetic test data. It is kept outside
        // SQLite and supplied explicitly after reopen; it is never reconstructed from metadata.
        let input = mapped_input(response.clone())?;
        let decision = action_decision();
        let mut source = ProbeSource {
            calls: 0,
            decision: decision.clone(),
            durable: durable.clone(),
            context_status_at_call: None,
            reservation_present_at_call: false,
        };
        let mut recorder = DecisionRecorder::with_durable(
            &mut source,
            TelemetryHandle::disabled(),
            durable.clone(),
        );
        assert_eq!(recorder.decide(&input)?, decision);
        drop(recorder);
        assert_eq!(source.calls, 1);
        assert_eq!(
            source.context_status_at_call,
            Some(RecipeInvocationStatus::ContextValidated)
        );
        assert!(source.reservation_present_at_call);
        let reservation_before = durable
            .store
            .try_borrow()?
            .provider_reservation("provider-reservation-model-execution-1")?;
        let stored_before = durable.store.try_borrow()?.decision("model-execution-1")?;
        assert!(stored_before.completed);
        assert_eq!(
            invocation_status(&durable)?,
            RecipeInvocationStatus::ContextValidated
        );
        drop(source);
        durable.close()?;
        drop(durable);

        let store = ExecutionStore::open(ExecutionStoreConfig::new(path.clone()))?;
        let changed_build = ExecutionFingerprint::new(
            "seed-runtime-map-1",
            "f".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
            "d".repeat(64),
        )?;
        let changed_config = ExecutionFingerprint::new(
            "seed-runtime-map-1",
            "a".repeat(64),
            "b".repeat(64),
            "f".repeat(64),
            "d".repeat(64),
        )?;
        assert!(matches!(
            store.resume_episode(&lineage.episode_id, &changed_build)?,
            sts2_harness::ResumeState::ReconstructionRequired { .. }
        ));
        assert!(matches!(
            store.resume_episode(&lineage.episode_id, &changed_config)?,
            sts2_harness::ResumeState::ReconstructionRequired { .. }
        ));
        assert!(matches!(
            store.resume_episode(&lineage.episode_id, &fingerprint)?,
            sts2_harness::ResumeState::Ready { .. }
        ));
        let reopened = DurableHandle::from_store_for_lifecycle_test(
            store,
            lineage,
            fingerprint,
            String::from("model-revision-1"),
            "e".repeat(64),
        )?;
        let reopened_reads = Cell::new(0_u8);
        let repeated_read = collect_map_snapshot(
            &reopened,
            "state-1",
            1,
            ModelExecutionId::new(1).ok_or("execution identity must be nonzero")?,
            &actions,
            || {
                reopened_reads.set(reopened_reads.get() + 1);
                Ok(response.clone())
            },
        );
        assert!(repeated_read.is_err());
        assert_eq!(reopened_reads.get(), 0);

        let mut reopened_source = ProbeSource {
            calls: 0,
            decision: Decision::Wait {
                rationale: String::from("completed decision must be reused"),
            },
            durable: reopened.clone(),
            context_status_at_call: None,
            reservation_present_at_call: false,
        };
        let mut reopened_recorder = DecisionRecorder::with_durable(
            &mut reopened_source,
            TelemetryHandle::disabled(),
            reopened.clone(),
        );
        assert_eq!(reopened_recorder.decide(&input)?, decision);
        assert_eq!(
            reopened_recorder.decide(&no_context_input()?),
            Err(PolicyError::InputBlocked)
        );
        drop(reopened_recorder);
        assert_eq!(reopened_source.calls, 0);
        assert_eq!(
            invocation_status(&reopened)?,
            RecipeInvocationStatus::ContextValidated
        );
        let reservation_after = reopened
            .store
            .try_borrow()?
            .provider_reservation("provider-reservation-model-execution-1")?;
        let stored_after = reopened.store.try_borrow()?.decision("model-execution-1")?;
        assert_eq!(reservation_after, reservation_before);
        assert_eq!(stored_after, stored_before);
        drop(reopened_source);
        reopened.close()?;
        Ok::<(), Box<dyn std::error::Error>>(())
    })();
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    result
}

#[test]
fn pending_and_cold_receipt_still_block_reference_only_admission()
-> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join(format!(
        "sts2-runtime-map-reference-receipt-{}-{}.sqlite3",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let (durable, lineage, fingerprint) = durable_fixture_with_store(ExecutionStore::open(
            ExecutionStoreConfig::new(path.clone()),
        )?)?;
        durable.begin_map_invocation(invocation_binding(&durable)?)?;
        assert_eq!(
            invocation_status(&durable)?,
            RecipeInvocationStatus::IntentRecorded
        );
        let input = no_map_combat_input()?;
        let refusal =
            String::from("runtime map invocation has no owner-validated decision context");
        assert_eq!(
            finalize_decision_context(&durable, &input, &"f".repeat(64)),
            Err(refusal.clone())
        );
        assert!(
            durable
                .store
                .try_borrow()?
                .provider_reservation("provider-reservation-model-execution-1")
                .is_err()
        );
        durable.close()?;
        drop(durable);

        let store = ExecutionStore::open(ExecutionStoreConfig::new(path.clone()))?;
        let reopened = DurableHandle::from_store_for_lifecycle_test(
            store,
            lineage,
            fingerprint,
            String::from("model-revision-1"),
            String::from("config:selected"),
        )?;
        assert_eq!(
            finalize_decision_context(&reopened, &input, &"f".repeat(64)),
            Err(refusal)
        );
        assert_eq!(
            reopened.decision_admission_with_reuse(&input).err(),
            Some(String::from(
                "runtime map invocation has no owner-validated decision context",
            ))
        );
        let lineage = reopened.lifecycle_lineage();
        assert!(
            reopened
                .store
                .try_borrow_mut()?
                .recipe_invocation_exists_for_execution(&lineage, input.execution_id)?
        );
        assert!(
            reopened
                .store
                .try_borrow()?
                .provider_reservation("provider-reservation-model-execution-1")
                .is_err()
        );
        reopened.close()?;
        drop(reopened);
        Ok::<(), Box<dyn std::error::Error>>(())
    })();
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    result
}
