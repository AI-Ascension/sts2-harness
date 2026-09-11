// SPDX-License-Identifier: MIT

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn shared_model_execution_replays_each_step_with_its_fresh_semantic_payload() {
        use sts2_harness::{ActionKind, EpisodeLegalAction, EpisodeStage};
        let make = |generation: u64, card: &str, id: &str| {
            json!({
                "state_id":format!("combat-{generation}"),"generation":generation,"visible_seed":"seed",
                "player":{"hp":50,"max_hp":50,"energy":3,"gold":0,"hand":[],"deck":[],"discard":[],"exhaust":[]},
                "state":{"state":"combat","turn_index":1,"enemies":[]},
                "legal_actions":[{"action_id":id,"action":{"kind":"play_card","card_id":card,"target_id":null}}]
            })
        };
        let records = ["a", "b"].iter().enumerate().map(|(index, card)| {
            let id = format!("recorded-{index}");
            json!({"event":"model_decision","model_execution_id":7,
                "reused_model_execution":index > 0,"action_id":id,"observation":make(index as u64,card,&id)})
        }).collect();
        let replay = Replay {
            records: Some(records),
            terminal: None,
        };
        for (index, card) in ["a", "b"].iter().enumerate() {
            let generation = index as u64 + 20;
            let id = format!("fresh-{index}");
            let observation = EpisodeObservation::new(
                format!("combat-{generation}"),
                generation,
                EpisodeStage::Combat,
                true,
                false,
                true,
                make(generation, card, &id),
            )
            .expect("fixture observation");
            let actions = EpisodeLegalActionSet::new(
                observation.state_id(),
                generation,
                vec![EpisodeLegalAction::new(&id, ActionKind::PlayCard).expect("fixture action")],
            )
            .expect("fixture catalog");
            assert!(matches!(replay.decide(index as u32,&observation,&actions),
                Ok(Some(Decision::Action {action_id,..})) if action_id == id));
            let wrong = EpisodeObservation::new(
                format!("combat-{generation}"),
                generation,
                EpisodeStage::Combat,
                true,
                false,
                true,
                make(generation, "wrong-target-card", &id),
            )
            .expect("fixture observation");
            assert!(replay.decide(index as u32, &wrong, &actions).is_err());
        }
    }

    #[test]
    fn replay_normalizes_only_observation_identity_and_catalog() {
        let first = json!({"legal_actions":[{"action_id":"old-id", "action":{"kind":"end_turn"}}]});
        let next =
            json!({"legal_actions":[{"action_id":"fresh-id", "action":{"kind":"end_turn"}}]});
        assert_eq!(
            action_payload(&first, "old-id"),
            action_payload(&next, "fresh-id")
        );
        assert!(action_payload(&next, "old-id").is_none());
        assert_ne!(
            canonical(json!({"visible_seed":"A","generation":1})),
            canonical(json!({"visible_seed":"B","generation":2}))
        );
        assert_ne!(
            canonical(json!({"player":{"hp":1}})),
            canonical(json!({"player":{"hp":2}}))
        );
    }

    #[test]
    fn digest_trajectory_rebinds_current_catalog_without_raw_observation() {
        use sts2_harness::{ActionKind, EpisodeLegalAction, EpisodeStage};
        let recorded_observation = EpisodeObservation::new(
            "combat-recorded",
            1,
            EpisodeStage::Combat,
            true,
            false,
            true,
            json!({
                "state_id":"combat-recorded","generation":1,"visible_seed":"seed",
                "player":{"hp":50,"max_hp":50,"energy":3,"gold":0,"hand":[],"deck":[],"discard":[],"exhaust":[]},
                "state":{"state":"combat","turn_index":1,"enemies":[]},
                "legal_actions":[{"action_id":"recorded-id","action":{"kind":"end_turn"}}]
            }),
        )
        .expect("recorded observation");
        let current_observation = EpisodeObservation::new(
            "combat-current",
            20,
            EpisodeStage::Combat,
            true,
            false,
            true,
            json!({
                "state_id":"combat-current","generation":20,"visible_seed":"seed",
                "player":{"hp":50,"max_hp":50,"energy":3,"gold":0,"hand":[],"deck":[],"discard":[],"exhaust":[]},
                "state":{"state":"combat","turn_index":1,"enemies":[]},
                "legal_actions":[{"action_id":"fresh-id","action":{"kind":"end_turn"}}]
            }),
        )
        .expect("current observation");
        let action = EpisodeLegalAction::new("fresh-id", ActionKind::EndTurn).expect("action");
        let actions = EpisodeLegalActionSet::new(
            current_observation.state_id(),
            current_observation.generation(),
            vec![action],
        )
        .expect("action set");
        let recorded_payload =
            action_payload(recorded_observation.fair_play().as_value(), "recorded-id")
                .expect("recorded payload");
        let replay = Replay {
            records: Some(vec![json!({
                "observation_digest":Replay::observation_digest(&recorded_observation),
                "action_digest":Replay::action_digest(recorded_payload)
            })]),
            terminal: None,
        };
        assert!(matches!(
            replay.decide(0, &current_observation, &actions),
            Ok(Some(Decision::Action { action_id, .. })) if action_id == "fresh-id"
        ));
        let changed = EpisodeObservation::new(
            "combat-current",
            20,
            EpisodeStage::Combat,
            true,
            false,
            true,
            json!({
                "state_id":"combat-current","generation":20,"visible_seed":"different-seed",
                "player":{"hp":50,"max_hp":50,"energy":3,"gold":0,"hand":[],"deck":[],"discard":[],"exhaust":[]},
                "state":{"state":"combat","turn_index":1,"enemies":[]},
                "legal_actions":[{"action_id":"fresh-id","action":{"kind":"end_turn"}}]
            }),
        )
        .expect("changed observation");
        assert!(replay.decide(0, &changed, &actions).is_err());
    }
}
