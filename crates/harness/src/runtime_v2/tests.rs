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
}
