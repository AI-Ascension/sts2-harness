// SPDX-License-Identifier: MIT

use super::*;
use crate::management::{
    ContextOwnerControlLimits, ContextRenderSource, ContextRenderSourceIdentity,
    LiveContextRenderPort,
};
use crate::{
    ContextBoundary, ContextDraft, ContextItem, ContextItemRef, ContextRenderLimits,
    ContextSourceDocument, ExoConfig, ExoDecisionSource, ExoProvider, ExoSession, ExoTransport,
    ExoTransportError,
};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct RenderState {
    source: ContextRenderSource,
}

struct RenderPort {
    state: Arc<Mutex<RenderState>>,
    stale: Arc<std::sync::atomic::AtomicBool>,
    limits: ContextRenderLimits,
}

impl LiveContextRenderPort for RenderPort {
    fn render_source_for_decision(
        &self,
        _actor: &AuthContext,
        _request: &RunRequest,
        _definition_digest: &str,
        _binding: &RuntimeAuthorityBinding,
        _control_limits: &ContextOwnerControlLimits,
        _input: &DecisionInput,
        _context_ref: &str,
    ) -> Result<ContextRenderSource, ManagementError> {
        let state = self.state.lock().map_err(|_| {
            ManagementError::unavailable("test_render_lock", "render fixture lock was poisoned")
        })?;
        let mut source = state.source.clone();
        source.limits = self.limits;
        Ok(source)
    }

    fn assert_render_source_current(
        &self,
        _actor: &AuthContext,
        _request: &RunRequest,
        _definition_digest: &str,
        _binding: &RuntimeAuthorityBinding,
        _control_limits: &ContextOwnerControlLimits,
        _input: &DecisionInput,
        _context_ref: &str,
        expected: &ContextRenderSourceIdentity,
    ) -> Result<(), ManagementError> {
        let state = self.state.lock().map_err(|_| {
            ManagementError::unavailable("test_render_lock", "render fixture lock was poisoned")
        })?;
        if self.stale.load(Ordering::SeqCst) || &state.source.identity != expected {
            return Err(ManagementError::conflict(
                "context_render_source_stale",
                "test owner source identity changed during inference",
            ));
        }
        Ok(())
    }

    fn render_required(&self) -> bool {
        true
    }
}

struct PreparedRecordingTransport {
    exchanges: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
    on_exchange: Option<Arc<dyn Fn() + Send + Sync>>,
    responses: Option<Arc<Mutex<VecDeque<Vec<u8>>>>>,
}

impl ExoTransport for PreparedRecordingTransport {
    fn exchange(
        &mut self,
        request: &[u8],
        _max_response_bytes: usize,
        _timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        self.exchanges.fetch_add(1, Ordering::SeqCst);
        self.requests
            .lock()
            .map_err(|_| ExoTransportError::Unavailable)?
            .push(request.to_vec());
        if let Some(on_exchange) = &self.on_exchange {
            on_exchange();
        }
        self.responses
            .as_ref()
            .map(|responses| {
                responses
                    .lock()
                    .map_err(|_| ExoTransportError::Unavailable)?
                    .pop_front()
                    .ok_or(ExoTransportError::MalformedResponse)
            })
            .unwrap_or_else(|| {
                Ok(
                    br#"{"decision":"action","action_id":"combat.end-turn","rationale":"bounded managed decision"}"#
                        .to_vec(),
                )
            })
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        Ok(())
    }
}

fn render_fixture() -> (ContextRenderSource, ExoConfig) {
    let mut items = BTreeMap::new();
    let mut draft = ContextDraft::new("draft-1", "revision-1");
    for (item_id, bytes) in [
        ("strategy-1", b"trusted retained strategy".to_vec()),
        ("strategy-2", b"trusted retained objective".to_vec()),
    ] {
        let item = ContextItem {
            reference: ContextItemRef {
                item_id: item_id.to_owned(),
                version: 1,
                sha256: crate::sha256_hex(&bytes),
            },
            kind: "strategy".to_owned(),
            bytes,
            protected: false,
            expires_at: 100,
        };
        draft.selected_items.push(item.reference.clone());
        items.insert(format!("{item_id}:1"), item);
    }
    let document = ContextSourceDocument { draft, items };
    let boundary = ContextBoundary {
        run_id: "run-1".to_owned(),
        episode_id: "episode-1".to_owned(),
        agent_id: "test-agent".to_owned(),
        state_id: "combat-1".to_owned(),
        generation: 1,
        observation_sha256: "a".repeat(64),
        catalog_sha256: "b".repeat(64),
        adapter_revision: crate::EXO_SOURCE_REVISION.to_owned(),
        model_revision: "test-model".to_owned(),
        configuration_sha256: "c".repeat(64),
        output_schema_sha256: "d".repeat(64),
        controller_epoch: 1,
        gate_epoch: 0,
        control_version: 1,
    };
    let identity = ContextRenderSourceIdentity {
        owner_id: "test-owner".to_owned(),
        owner_version: "v1".to_owned(),
        catalog_digest: "e".repeat(64),
        binding_id: "binding-1".to_owned(),
        binding_version: 1,
        binding_digest: "f".repeat(64),
        invocation_id: "invocation-1".to_owned(),
        instance_id: "test-instance".to_owned(),
        lease_id: "test-lease".to_owned(),
        lease_epoch: 1,
        active_revision_id: "revision-1".to_owned(),
        source_id: "strategy".to_owned(),
        source_version: 1,
        source_digest: "9".repeat(64),
        boundary: boundary.clone(),
    };
    let source = ContextRenderSource {
        source_id: "strategy".to_owned(),
        source_version: 1,
        source_digest: "9".repeat(64),
        active_revision_id: "revision-1".to_owned(),
        boundary,
        limits: ContextRenderLimits::harness_maxima(),
        document,
        now: 1,
        valid_until: 100,
        identity,
    };
    let config = ExoConfig::new(
        "b06869ab789dee3f80ca474b5fa89dbe47ccb859",
        64 * 1024,
        1024,
        1_000,
    )
    .expect("Exo config");
    (source, config)
}

type RenderTestSession = (
    ProductionLiveWorkflowSession,
    Arc<Mutex<RenderState>>,
    Arc<AtomicUsize>,
    Arc<Mutex<Vec<Vec<u8>>>>,
);

fn render_test_session(
    source: ContextRenderSource,
    config: ExoConfig,
    limits: ContextRenderLimits,
    stale: Arc<std::sync::atomic::AtomicBool>,
    on_exchange: Option<Arc<dyn Fn() + Send + Sync>>,
) -> RenderTestSession {
    let (mut session, _, _) = make_session(Change::HistoryOnlyDuringInference);
    let render_state = Arc::new(Mutex::new(RenderState { source }));
    session.context_render = Some(Arc::new(RenderPort {
        state: Arc::clone(&render_state),
        stale,
        limits,
    }));
    session.context_control_limits = Some(ContextOwnerControlLimits {
        schema_version: crate::management::CONTEXT_OWNER_CONTROL_LIMITS_SCHEMA.to_owned(),
        owner_id: "test-owner".to_owned(),
        owner_version: "v1".to_owned(),
        catalog_digest: "e".repeat(64),
        max_control_events: 32,
    });
    let exchanges = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let transport = PreparedRecordingTransport {
        exchanges: Arc::clone(&exchanges),
        requests: Arc::clone(&requests),
        on_exchange,
        responses: None,
    };
    let provider = ExoDecisionSource::new(ExoSession::new(ExoProvider::new(transport, config)));
    session.provider = Some(Box::new(provider));
    (session, render_state, exchanges, requests)
}

fn selected_limits(max_items: usize) -> ContextRenderLimits {
    ContextRenderLimits {
        max_items,
        ..ContextRenderLimits::harness_maxima()
    }
}

fn managed_plan_input(generation: u64) -> DecisionInput {
    let has_card = generation == 1;
    let mut legal_actions = Vec::new();
    let mut actions = Vec::new();
    if has_card {
        legal_actions.push(serde_json::json!({
            "action_id":"combat.play-card",
            "action":{"kind":"play_card","card_id":"card-1","target_id":"enemy-1"}
        }));
        actions.push(
            EpisodeLegalAction::new("combat.play-card", crate::ActionKind::PlayCard)
                .expect("play-card"),
        );
    }
    legal_actions.push(serde_json::json!({
        "action_id":"combat.end-turn",
        "action":{"kind":"end_turn"}
    }));
    actions.push(
        EpisodeLegalAction::new("combat.end-turn", crate::ActionKind::EndTurn).expect("end-turn"),
    );
    let observation = EpisodeObservation::new(
        "combat-1",
        generation,
        EpisodeStage::Combat,
        true,
        false,
        true,
        serde_json::json!({
            "state_id":"combat-1",
            "generation":generation,
            "visible_seed":"fixture",
            "player":{
                "hp":1,"max_hp":1,"energy":1,"gold":0,
                "hand":if has_card { serde_json::json!([{"card_id":"card-1","name":"Strike","cost":1,"upgraded":false}]) } else { serde_json::json!([]) },
                "deck":[],"discard":[],"exhaust":[]
            },
            "state":{"state":"combat","turn_index":1,"enemies":[]},
            "legal_actions":legal_actions
        }),
    )
    .expect("managed plan observation");
    let actions = EpisodeLegalActionSet::new("combat-1", generation, actions)
        .expect("managed plan legal actions");
    DecisionInput::new(
        crate::ModelExecutionId::new(generation).expect("managed plan execution"),
        observation,
        actions,
        "test objective",
        Vec::new(),
    )
}

#[test]
fn served_managed_render_refuses_selected_item_overage_before_provider_exchange() {
    let (source, config) = render_fixture();
    let (mut session, _, exchanges, _) =
        render_test_session(source, config, selected_limits(1), Default::default(), None);

    let error = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("selected max_items=1 must reject two retained items");

    assert_eq!(error.code, "context_render_limit_exceeded");
    assert!(error.message.contains("max_items"));
    assert_eq!(exchanges.load(Ordering::SeqCst), 0);
}

#[test]
fn served_managed_render_sends_the_exact_prepared_bytes_once() {
    let (source, config) = render_fixture();
    let expected = {
        let input = input();
        let managed = crate::context_control::ManagedRenderInput {
            execution_id: input.execution_id.to_string(),
            state_id: input.observation.state_id().to_owned(),
            generation: input.observation.generation(),
            observation: input.observation.fair_play().as_value().clone(),
            legal_action_ids: input
                .legal_actions
                .actions()
                .iter()
                .map(|action| action.action_id().to_owned())
                .collect(),
            objective: input.objective.clone(),
            hard_constraints: input.hard_constraints.clone(),
            map_context: None,
        };
        crate::context_control::ContextRenderer::enabled_at_with_limits(
            &source.boundary,
            managed,
            &source.document.draft,
            &source.document.items,
            &config,
            source.now,
            &selected_limits(2),
        )
        .expect("selected source prepares")
        .provider_bytes()
        .to_vec()
    };
    let (mut session, _, exchanges, requests) =
        render_test_session(source, config, selected_limits(2), Default::default(), None);

    let decision = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect("prepared managed decision");

    assert!(matches!(decision, Decision::Action { .. }));
    assert_eq!(exchanges.load(Ordering::SeqCst), 1);
    assert_eq!(
        requests.lock().expect("recorded requests").as_slice(),
        &[expected]
    );
}

#[test]
fn served_managed_render_discards_result_when_source_changes_during_inference() {
    let (source, config) = render_fixture();
    let stale = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let callback_stale = Arc::clone(&stale);
    let on_exchange: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        callback_stale.store(true, Ordering::SeqCst);
    });
    let (mut session, _, exchanges, _) = render_test_session(
        source,
        config,
        selected_limits(2),
        Arc::clone(&stale),
        Some(on_exchange),
    );

    let error = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("changed source must fence the provider result");

    assert_eq!(error.code, "context_render_source_stale");
    assert_eq!(exchanges.load(Ordering::SeqCst), 1);
    assert!(stale.load(Ordering::SeqCst));
}

#[cfg(test)]
#[path = "production_managed_plan_tests.rs"]
mod plan_tests;
