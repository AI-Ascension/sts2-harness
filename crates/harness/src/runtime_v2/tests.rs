// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_and_observation_are_frozen_and_bounded() {
        assert_eq!(RuntimeV2Action::end_turn().action_id(), ACTION_ID);
        assert!(RuntimeV2Action::new("use_budget").is_err());
        assert!(
            RuntimeV2Observation::new(
                RuntimeV2CombatPhase::PlayerTurn,
                RUNTIME_V2_MAX_TURN_INDEX + 1,
                true,
                INITIAL_GENERATION,
            )
            .is_err()
        );
    }

    #[test]
    fn golden_shape_round_trips_with_the_handoff_digest() -> Result<(), RuntimeV2Error> {
        let context = RuntimeV2Context::new(INSTANCE_ID, SESSION_ID, LEASE_ID, LEASE_EPOCH)?;
        let message = RuntimeV2Message::action_request(
            &context,
            "corr-0002",
            INITIAL_GENERATION,
            RuntimeV2OperationId::new("op-1")?,
            RuntimeV2Action::end_turn(),
        )?;
        let encoded = message.to_json()?;
        assert!(encoded.contains("\"protocol_version\":\"runtime-v2\""));
        assert!(encoded.contains(RUNTIME_V2_SCHEMA_DIGEST));
        assert!(encoded.contains("\"operation_id\":\"op-1\""));
        assert!(encoded.contains("\"action_id\":\"end_turn\""));
        assert_eq!(RuntimeV2Message::from_json(&encoded)?, message);
        Ok(())
    }

    #[test]
    fn fake_runner_has_one_mutation_and_all_lifecycle_outcomes() -> Result<(), RuntimeV2Error> {
        let report = run_runtime_v2_fake_trace()?;
        assert_eq!(report.evidence().initial_generation(), INITIAL_GENERATION);
        assert_eq!(
            report.evidence().settled_generation(),
            INITIAL_GENERATION + 1
        );
        assert_eq!(report.evidence().mutation_count(), 1);
        assert!(
            report
                .evidence()
                .duplicate_replay_without_second_application()
        );
        assert!(report.evidence().stale_epoch_rejected());
        assert!(report.evidence().no_blind_retry_after_disconnect());
        let events = report
            .trajectory()
            .records()
            .iter()
            .map(RuntimeV2Record::event_kind)
            .collect::<Vec<_>>();
        assert!(events.contains(&RuntimeV2EventKind::Requested));
        assert!(events.contains(&RuntimeV2EventKind::Accepted));
        assert!(events.contains(&RuntimeV2EventKind::Settled));
        assert!(events.contains(&RuntimeV2EventKind::Rejected));
        assert!(events.contains(&RuntimeV2EventKind::Unknown));
        assert!(events.contains(&RuntimeV2EventKind::Reconciled));
        assert!(report.artifact().schema_bytes_verified());
        assert_eq!(
            report.artifact().schema_bytes_digest(),
            RUNTIME_V2_SCHEMA_DIGEST
        );
        Ok(())
    }

    #[test]
    fn dispatch_rechecks_an_intervening_state_before_mutation() -> Result<(), RuntimeV2Error> {
        let context = RuntimeV2Context::new(INSTANCE_ID, SESSION_ID, LEASE_ID, LEASE_EPOCH)?;
        let initial = RuntimeV2Observation::new(
            RuntimeV2CombatPhase::PlayerTurn,
            INITIAL_TURN_INDEX,
            true,
            INITIAL_GENERATION,
        )?;
        let mut fake = FakeRuntimeV2::new(context.clone(), initial);
        let operation_id = RuntimeV2OperationId::new("op-intervening-state")?;
        let action = RuntimeV2Action::end_turn();
        let request = RuntimeV2Message::action_request(
            &context,
            "corr-intervening-request",
            INITIAL_GENERATION,
            operation_id.clone(),
            action.clone(),
        )?;
        assert_eq!(
            fake.accept_action(&request)?.message.status(),
            Some(RuntimeV2Status::Accepted)
        );

        fake.intervene_with_observation(RuntimeV2Observation::new(
            RuntimeV2CombatPhase::EnemyTurn,
            INITIAL_TURN_INDEX,
            true,
            INITIAL_GENERATION,
        )?);
        let rejected = match fake.disconnect_after_write(&operation_id, &request) {
            Ok(DispatchOutcome::Rejected(message)) => message,
            Err(_) => {
                return Err(RuntimeV2Error::Invalid(
                    "intervening state was not fenced before dispatch",
                ));
            }
        };
        assert_eq!(rejected.status(), Some(RuntimeV2Status::Rejected));
        assert_eq!(rejected.error_code(), Some("sts2.game-core/not_player_turn"));
        assert_eq!(rejected.correlation_id(), "corr-intervening-request");
        assert_eq!(fake.mutation_count(), 0);

        let replay_request = RuntimeV2Message::action_request(
            &context,
            "corr-intervening-replay",
            INITIAL_GENERATION,
            operation_id.clone(),
            action,
        )?;
        let replay = fake.accept_action(&replay_request)?;
        assert!(replay.replayed);
        assert_eq!(replay.message.correlation_id(), "corr-intervening-replay");
        assert_eq!(
            replay.message.operation_id().map(|id| id.as_str()),
            Some("op-intervening-state")
        );
        assert_eq!(replay.message.status(), Some(RuntimeV2Status::Rejected));
        assert_eq!(fake.mutation_count(), 0);
        Ok(())
    }

    #[test]
    fn settled_and_terminal_replays_rebind_correlation_with_full_context() -> Result<(), RuntimeV2Error>
    {
        let context = RuntimeV2Context::new(INSTANCE_ID, SESSION_ID, LEASE_ID, LEASE_EPOCH)?;
        let initial = RuntimeV2Observation::new(
            RuntimeV2CombatPhase::PlayerTurn,
            INITIAL_TURN_INDEX,
            true,
            INITIAL_GENERATION,
        )?;
        let mut fake = FakeRuntimeV2::new(context.clone(), initial);
        let operation_id = RuntimeV2OperationId::new("op-replay")?;
        let action = RuntimeV2Action::end_turn();
        let request = RuntimeV2Message::action_request(
            &context,
            "corr-original",
            INITIAL_GENERATION,
            operation_id.clone(),
            action.clone(),
        )?;
        fake.accept_action(&request)?;
        assert_eq!(
            fake.disconnect_after_write(&operation_id, &request),
            Err(RuntimeV2Error::PostWriteDisconnect)
        );
        let reconcile = RuntimeV2Message::reconcile_request(
            &context,
            "corr-reconcile",
            INITIAL_GENERATION,
            operation_id.clone(),
        )?;
        assert_eq!(
            fake.reconcile_action(&reconcile)?.message.status(),
            Some(RuntimeV2Status::Settled)
        );

        let duplicate_request = RuntimeV2Message::action_request(
            &context,
            "corr-new-duplicate",
            INITIAL_GENERATION,
            operation_id.clone(),
            action.clone(),
        )?;
        let duplicate = fake.accept_action(&duplicate_request)?;
        assert!(duplicate.replayed);
        assert_eq!(duplicate.message.correlation_id(), "corr-new-duplicate");
        assert_eq!(
            duplicate.message.operation_id().map(|id| id.as_str()),
            Some("op-replay")
        );
        assert_eq!(duplicate.message.instance_id(), INSTANCE_ID);
        assert_eq!(duplicate.message.session_id(), SESSION_ID);
        assert_eq!(duplicate.message.lease_id(), LEASE_ID);
        assert_eq!(duplicate.message.lease_epoch(), LEASE_EPOCH);
        assert_eq!(fake.mutation_count(), 1);

        let conflicting_context =
            RuntimeV2Context::new("other-instance", SESSION_ID, LEASE_ID, LEASE_EPOCH)?;
        let conflict_request = RuntimeV2Message::action_request(
            &conflicting_context,
            "corr-conflict",
            INITIAL_GENERATION,
            operation_id,
            action.clone(),
        )?;
        let conflict = fake.accept_action(&conflict_request)?;
        assert!(!conflict.replayed);
        assert_eq!(conflict.message.status(), Some(RuntimeV2Status::Rejected));
        assert_eq!(conflict.message.error_code(), Some("idempotency_conflict"));
        assert_eq!(fake.mutation_count(), 1);

        let terminal_operation = RuntimeV2OperationId::new("op-terminal-replay")?;
        let stale_context = RuntimeV2Context::new(INSTANCE_ID, SESSION_ID, LEASE_ID, 0)?;
        let terminal_request = RuntimeV2Message::action_request(
            &stale_context,
            "corr-terminal-original",
            INITIAL_GENERATION + 1,
            terminal_operation.clone(),
            action,
        )?;
        let terminal = fake.accept_action(&terminal_request)?;
        assert_eq!(terminal.message.status(), Some(RuntimeV2Status::Rejected));
        assert_eq!(terminal.message.error_code(), Some(STALE_EPOCH_ERROR));
        let terminal_replay_request = RuntimeV2Message::action_request(
            &stale_context,
            "corr-terminal-new",
            INITIAL_GENERATION + 1,
            terminal_operation,
            RuntimeV2Action::end_turn(),
        )?;
        let terminal_replay = fake.accept_action(&terminal_replay_request)?;
        assert!(terminal_replay.replayed);
        assert_eq!(terminal_replay.message.correlation_id(), "corr-terminal-new");
        assert_eq!(terminal_replay.message.status(), Some(RuntimeV2Status::Rejected));
        assert_eq!(terminal_replay.message.error_code(), Some(STALE_EPOCH_ERROR));
        assert_eq!(fake.mutation_count(), 1);
        Ok(())
    }
}
