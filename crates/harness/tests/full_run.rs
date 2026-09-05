// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use serde_json::{Value, json};
use sts2_harness::{
    ActionKind, Decision, DecisionInput, DecisionSource, EpisodeLegalAction, EpisodeLegalActionSet,
    EpisodeMachine, EpisodeMachineError, EpisodeObservation, EpisodePhase, EpisodeStage,
    ModelExecutionId, NoncombatCoordinator, NoncombatStage, PolicyChoice, PolicyError,
    PolicyRouter, RunSetupCoordinator,
};

#[derive(Default)]
struct RecordingDecisionSource {
    states: Vec<String>,
}

impl DecisionSource for RecordingDecisionSource {
    fn decide(&mut self, input: &DecisionInput) -> Result<Decision, PolicyError> {
        let action = input
            .legal_actions
            .actions()
            .first()
            .ok_or(PolicyError::IllegalAction)?;
        self.states
            .push(input.observation.stage().name().to_owned());
        Ok(Decision::Action {
            action_id: action.action_id().to_owned(),
            rationale: String::from("provider selected a current host action"),
            confidence: Some(80),
        })
    }
}

fn observation(stage: EpisodeStage, generation: u64) -> EpisodeObservation {
    let state_id = format!("{}-{generation}", stage.name());
    EpisodeObservation::new(
        state_id,
        generation,
        stage,
        true,
        false,
        true,
        fair_play(stage, generation),
    )
    .expect("test projection is valid")
}

fn fair_play(stage: EpisodeStage, generation: u64) -> Value {
    let state = match stage {
        EpisodeStage::Setup => json!({"state":"setup","characters":[]}),
        EpisodeStage::Map => json!({"state":"map","node_id":"node-1","options":[]}),
        EpisodeStage::Combat => json!({"state":"combat","turn_index":1,"enemies":[]}),
        EpisodeStage::Reward => json!({"state":"reward","options":[]}),
        EpisodeStage::Shop => json!({"state":"shop","items":[]}),
        EpisodeStage::Event | EpisodeStage::Selection => json!({"state":stage.name(),"choices":[]}),
        EpisodeStage::Rest => json!({"state":"rest","options":[]}),
        EpisodeStage::Victory => json!({"state":"victory"}),
        EpisodeStage::Defeat => json!({"state":"defeat","reason":"test"}),
        EpisodeStage::Recovery => json!({"state":"recovery","code":"test"}),
        EpisodeStage::Unknown => json!({"state":"recovery","code":"test"}),
    };
    json!({
        "state_id": format!("{}-{generation}", stage.name()),
        "generation": generation,
        "visible_seed": "visible-seed-only",
        "player": {"hp":50,"max_hp":50,"energy":3,"gold":99,"hand":[],"deck":[],"discard":[],"exhaust":[]},
        "state": state,
        "legal_actions": [{"action_id": format!("{}-action", stage.name()), "action": {"kind":"end_turn"}}]
    })
}

fn legal_actions(stage: EpisodeStage, generation: u64) -> EpisodeLegalActionSet {
    let (kind, suffix) = match stage {
        EpisodeStage::Setup => (ActionKind::StartRun, "start"),
        EpisodeStage::Map => (ActionKind::SelectMapNode, "node"),
        EpisodeStage::Combat => (ActionKind::EndTurn, "end-turn"),
        EpisodeStage::Reward => (ActionKind::ChooseReward, "reward"),
        EpisodeStage::Shop => (ActionKind::ShopPurchase, "purchase"),
        EpisodeStage::Event => (ActionKind::EventChoice, "choice"),
        EpisodeStage::Rest => (ActionKind::Rest, "rest"),
        EpisodeStage::Selection => (ActionKind::SelectCard, "card"),
        _ => (ActionKind::SaveQuit, "save-quit"),
    };
    EpisodeLegalActionSet::new(
        format!("{}-{generation}", stage.name()),
        generation,
        vec![
            EpisodeLegalAction::new(format!("{}-{suffix}", stage.name()), kind)
                .expect("test action is valid"),
        ],
    )
    .expect("test catalog is valid")
}

#[test]
fn full_run_routes_every_playable_surface_to_the_provider_and_tracks_terminals() {
    assert_provider_routes_playable_stages();
    assert_transition_state();
    assert_terminal_states();
    assert_recovery_state();
}

fn assert_provider_routes_playable_stages() {
    let mut source = RecordingDecisionSource::default();
    for (index, (stage, noncombat)) in playable_stages().iter().copied().enumerate() {
        let generation = index as u64;
        let choice = choose_stage(
            &mut source,
            stage,
            noncombat,
            generation,
            observation(stage, generation),
            legal_actions(stage, generation),
        )
        .expect("provider supplies a current action");
        assert!(matches!(choice, PolicyChoice::Action { .. }));
    }
    assert_eq!(
        source.states,
        vec![
            String::from("setup"),
            String::from("map"),
            String::from("combat"),
            String::from("reward"),
            String::from("shop"),
            String::from("event"),
            String::from("rest"),
            String::from("selection"),
        ]
    );
}

fn playable_stages() -> [(EpisodeStage, Option<NoncombatStage>); 8] {
    [
        (EpisodeStage::Setup, None),
        (EpisodeStage::Map, Some(NoncombatStage::Map)),
        (EpisodeStage::Combat, None),
        (EpisodeStage::Reward, Some(NoncombatStage::Reward)),
        (EpisodeStage::Shop, Some(NoncombatStage::Shop)),
        (EpisodeStage::Event, Some(NoncombatStage::Event)),
        (EpisodeStage::Rest, Some(NoncombatStage::Rest)),
        (EpisodeStage::Selection, Some(NoncombatStage::Selection)),
    ]
}

fn choose_stage(
    source: &mut RecordingDecisionSource,
    stage: EpisodeStage,
    noncombat: Option<NoncombatStage>,
    generation: u64,
    observation: EpisodeObservation,
    actions: EpisodeLegalActionSet,
) -> Result<PolicyChoice, PolicyError> {
    let execution = ModelExecutionId::new(generation + 1).expect("execution ID is valid");
    match noncombat {
        Some(stage) => NoncombatCoordinator.choose(
            source,
            stage,
            execution,
            observation,
            actions,
            "finish the run",
            Vec::new(),
        ),
        None if stage == EpisodeStage::Setup => RunSetupCoordinator.choose(
            source,
            execution,
            observation,
            actions,
            "finish the run",
            Vec::new(),
        ),
        None => PolicyRouter::choose(
            source,
            &DecisionInput::new(
                execution,
                observation,
                actions,
                "finish the run",
                Vec::new(),
            ),
        ),
    }
}

fn assert_transition_state() {
    let mut machine = EpisodeMachine::new();
    machine
        .observe(observation(EpisodeStage::Setup, 0))
        .expect("setup is accepted");
    machine
        .begin_dispatch("setup-start")
        .expect("setup can dispatch");
    machine
        .settle(observation(EpisodeStage::Map, 1))
        .expect("map transition is fresh");
    assert!(matches!(machine.phase(), EpisodePhase::Ready(_)));
}

fn assert_terminal_states() {
    assert_terminal(EpisodeStage::Victory);
    assert_terminal(EpisodeStage::Defeat);
}

fn assert_terminal(stage: EpisodeStage) {
    let mut machine = EpisodeMachine::new();
    machine
        .observe(
            EpisodeObservation::new(
                format!("{}-2", stage.name()),
                2,
                stage,
                false,
                false,
                false,
                fair_play(stage, 2),
            )
            .expect("terminal observation is valid"),
        )
        .expect("terminal observation is accepted");
    assert_eq!(machine.phase(), &EpisodePhase::Complete(stage));
}

fn assert_recovery_state() {
    let mut recovery = EpisodeMachine::new();
    let result = recovery.observe(
        EpisodeObservation::new(
            "recovery-0",
            0,
            EpisodeStage::Recovery,
            false,
            true,
            false,
            fair_play(EpisodeStage::Recovery, 0),
        )
        .expect("recovery is a valid blocked observation"),
    );
    assert_eq!(result, Err(EpisodeMachineError::UnknownState));
    assert!(matches!(recovery.phase(), EpisodePhase::Recovering { .. }));
}

trait StageName {
    fn name(self) -> &'static str;
}

impl StageName for EpisodeStage {
    fn name(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Map => "map",
            Self::Combat => "combat",
            Self::Reward => "reward",
            Self::Shop => "shop",
            Self::Event => "event",
            Self::Rest => "rest",
            Self::Selection => "selection",
            Self::Victory => "victory",
            Self::Defeat => "defeat",
            Self::Recovery => "recovery",
            Self::Unknown => "unknown",
        }
    }
}
