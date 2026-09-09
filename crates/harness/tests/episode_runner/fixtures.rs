// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn state(stage: EpisodeStage, generation: u64) -> State {
    let state_id = format!("{}-{generation}", stage_name(stage));
    let action_id = format!("{}-action", stage_name(stage));
    let kind = match stage {
        EpisodeStage::Setup => ActionKind::StartRun,
        EpisodeStage::Map => ActionKind::SelectMapNode,
        EpisodeStage::Combat => ActionKind::EndTurn,
        EpisodeStage::Reward => ActionKind::ChooseReward,
        EpisodeStage::Shop => ActionKind::ShopPurchase,
        EpisodeStage::Event => ActionKind::EventChoice,
        EpisodeStage::Rest => ActionKind::Rest,
        EpisodeStage::Selection => ActionKind::SelectCard,
        EpisodeStage::Victory | EpisodeStage::Defeat => ActionKind::SaveQuit,
        EpisodeStage::Unknown | EpisodeStage::Recovery => ActionKind::SaveQuit,
    };
    let observation = EpisodeObservation::new(
        state_id.clone(),
        generation,
        stage,
        !stage.is_terminal(),
        false,
        !stage.is_terminal(),
        projection(&state_id, generation, stage),
    )
    .expect("state projection is valid");
    let action = EpisodeLegalAction::new(action_id, kind).expect("action is valid");
    let actions = EpisodeLegalActionSet::new(state_id, generation, vec![action])
        .expect("action set is valid");
    State {
        observation,
        actions,
    }
}

fn projection(state_id: &str, generation: u64, stage: EpisodeStage) -> Value {
    let state = match stage {
        EpisodeStage::Setup => json!({"state":"setup","characters":[]}),
        EpisodeStage::Map => json!({"state":"map","node_id":"node-1","options":[]}),
        EpisodeStage::Combat => json!({"state":"combat","turn_index":1,"enemies":[]}),
        EpisodeStage::Reward | EpisodeStage::Rest => {
            json!({"state":stage_name(stage),"options":[]})
        }
        EpisodeStage::Shop => json!({"state":"shop","items":[]}),
        EpisodeStage::Event | EpisodeStage::Selection => {
            json!({"state":stage_name(stage),"choices":[]})
        }
        EpisodeStage::Victory => json!({"state":"victory"}),
        EpisodeStage::Defeat => json!({"state":"defeat","reason":"test"}),
        EpisodeStage::Unknown | EpisodeStage::Recovery => {
            json!({"state":"recovery","code":"test"})
        }
    };
    json!({
        "state_id": state_id,
        "generation": generation,
        "visible_seed": "visible-seed-only",
        "player": {"hp":50,"max_hp":50,"energy":3,"gold":99,"hand":[],"deck":[],"discard":[],"exhaust":[]},
        "state": state,
        "legal_actions": [{"action_id": format!("{}-action", stage_name(stage)), "action": {"kind":"end_turn"}}]
    })
}

fn stage_name(stage: EpisodeStage) -> &'static str {
    match stage {
        EpisodeStage::Setup => "setup",
        EpisodeStage::Map => "map",
        EpisodeStage::Combat => "combat",
        EpisodeStage::Reward => "reward",
        EpisodeStage::Shop => "shop",
        EpisodeStage::Event => "event",
        EpisodeStage::Rest => "rest",
        EpisodeStage::Selection => "selection",
        EpisodeStage::Victory => "victory",
        EpisodeStage::Defeat => "defeat",
        EpisodeStage::Recovery => "recovery",
        EpisodeStage::Unknown => "unknown",
    }
}

pub(super) fn runner() -> EpisodeRunner {
    let barrier = StabilityBarrier::new(2, 1).expect("barrier is valid");
    let recovery = RecoveryController::new(1).expect("recovery is valid");
    EpisodeRunner::new(
        EpisodeRunnerConfig::new(
            16,
            barrier,
            recovery,
            "complete the run",
            vec![String::from("use only current host legal actions")],
        )
        .expect("runner configuration is valid"),
    )
}

pub(super) fn complete_states() -> Vec<State> {
    [
        EpisodeStage::Setup,
        EpisodeStage::Map,
        EpisodeStage::Combat,
        EpisodeStage::Reward,
        EpisodeStage::Shop,
        EpisodeStage::Event,
        EpisodeStage::Rest,
        EpisodeStage::Selection,
        EpisodeStage::Victory,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, stage)| state(stage, index as u64))
    .collect()
}
