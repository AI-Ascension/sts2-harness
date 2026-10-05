// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::super::LIVE_CONTEXT_REF;
use super::*;
use serde_json::json;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use sts2_harness::context_control::MembershipContinuity;
use sts2_harness::management::{
    ContextRenderSourceIdentity, INFERENCE_PROFILE_SCHEMA_VERSION, InferenceProfileBudgets,
    InferenceProfileContinuity, InferenceProfileDescriptor, InferenceProfileGrants,
    InferenceProfileState,
};
use sts2_harness::{
    ActionKind, ContextBoundary, ContextDraft, ContextItem, ContextItemRef, ContextRenderLimits,
    ContextSourceDocument, Decision, DecisionInput, EpisodeLegalAction, EpisodeLegalActionSet,
    EpisodeObservation, EpisodeStage, ExoConfig, ExoTransportError, ModelExecutionId,
};

struct RecordingTransport {
    exchanges: Arc<AtomicUsize>,
    responses: VecDeque<Vec<u8>>,
}

impl sts2_harness::ExoTransport for RecordingTransport {
    fn exchange(
        &mut self,
        _request: &[u8],
        _max_response_bytes: usize,
        _timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        self.exchanges.fetch_add(1, Ordering::SeqCst);
        self.responses
            .pop_front()
            .ok_or(ExoTransportError::MalformedResponse)
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        Ok(())
    }
}

fn descriptor() -> InferenceProfileDescriptor {
    InferenceProfileDescriptor {
        schema_version: INFERENCE_PROFILE_SCHEMA_VERSION.to_owned(),
        profile_id: LIVE_DECISION_PROFILE_ID.to_owned(),
        version: "1.0.0".to_owned(),
        digest: String::new(),
        adapter: LIVE_DECISION_ADAPTER.to_owned(),
        requested_model: "model-reviewed-1".to_owned(),
        resolved_model: None,
        prompt_revision: "a".repeat(64),
        settings_revision: "b".repeat(64),
        supported_settings: vec!["max_provider_calls".to_owned()],
        operations: vec!["decide".to_owned()],
        node_kinds: vec!["decide".to_owned()],
        context_compatibility: vec![LIVE_CONTEXT_REF.to_owned()],
        continuity: InferenceProfileContinuity {
            provider_session_continuity: false,
            survives_controller_restart: false,
        },
        effective_budgets: InferenceProfileBudgets {
            max_input_bytes: 128 * 1024,
            max_output_tokens: 2_000_000,
            max_provider_calls: 10_000,
        },
        grants: InferenceProfileGrants {
            select: true,
            edit: false,
        },
        state: InferenceProfileState::Available,
    }
    .seal()
    .expect("the test descriptor is valid")
}

fn admitted_profile() -> AdmittedInferenceProfileBinding {
    let descriptor = descriptor();
    AdmittedInferenceProfileBinding {
        binding: sts2_harness::management::InferenceProfileBinding {
            graph_id: "main".to_owned(),
            node_id: "decision-1".to_owned(),
            node_kind: "decide".to_owned(),
            profile_ref: format!(
                "{}:{}:{}",
                descriptor.profile_id, descriptor.version, descriptor.digest
            ),
            profile_id: descriptor.profile_id.clone(),
            version: descriptor.version.clone(),
            digest: descriptor.digest.clone(),
            adapter: descriptor.adapter.clone(),
            requested_model: descriptor.requested_model.clone(),
            resolved_model: descriptor.resolved_model.clone(),
        },
        descriptor,
    }
}

fn source_with_profile(
    profile: AdmittedInferenceProfileBinding,
) -> (ProfileBoundDecisionSource, Arc<AtomicUsize>, ExoConfig) {
    let exchanges = Arc::new(AtomicUsize::new(0));
    let config = ExoConfig::new(sts2_harness::EXO_SOURCE_REVISION, 64 * 1024, 1024, 1_000)
        .expect("test Exo config is valid");
    let transport = RecordingTransport {
        exchanges: Arc::clone(&exchanges),
        responses: VecDeque::from([
            br#"{"decision":"action","action_id":"combat.end-turn","rationale":"normal profile decision"}"#.to_vec(),
            br#"{"decision":"action","action_id":"combat.end-turn","rationale":"prepared profile decision"}"#.to_vec(),
        ]),
    };
    let source =
        ExoDecisionSource::new(ExoSession::new(ExoProvider::new(transport, config.clone())));
    (
        ProfileBoundDecisionSource {
            source: Box::new(source),
            profiles: vec![profile],
        },
        exchanges,
        config,
    )
}

fn input() -> DecisionInput {
    let observation_value = json!({
        "state_id": "combat-1",
        "generation": 0,
        "visible_seed": "visible-seed",
        "player": {
            "hp": 50,
            "max_hp": 50,
            "energy": 3,
            "gold": 0,
            "hand": [],
            "deck": [],
            "discard": [],
            "exhaust": []
        },
        "state": {"state": "combat", "turn_index": 1, "enemies": []},
        "legal_actions": [
            {"action_id": "combat.end-turn", "action": {"kind": "end_turn"}}
        ]
    });
    DecisionInput::new(
        ModelExecutionId::new(7).expect("nonzero execution ID"),
        EpisodeObservation::new(
            "combat-1",
            0,
            EpisodeStage::Combat,
            true,
            false,
            true,
            observation_value,
        )
        .expect("valid observation"),
        EpisodeLegalActionSet::new(
            "combat-1",
            0,
            vec![
                EpisodeLegalAction::new("combat.end-turn", ActionKind::EndTurn)
                    .expect("valid action"),
            ],
        )
        .expect("valid action set"),
        "survive this encounter",
        vec!["use only the host action catalog".to_owned()],
    )
}

fn context_source(config: &ExoConfig) -> ContextRenderSource {
    let bytes = b"trusted strategy material".to_vec();
    let item = ContextItem {
        reference: ContextItemRef {
            item_id: "strategy-1".to_owned(),
            version: 1,
            sha256: sts2_harness::sha256_hex(&bytes),
        },
        kind: "strategy".to_owned(),
        bytes,
        protected: false,
        expires_at: 100,
    };
    let mut draft = ContextDraft::new("draft-1", "revision-1");
    draft.selected_items.push(item.reference.clone());
    let mut items = BTreeMap::new();
    items.insert("strategy-1:1".to_owned(), item);
    let boundary = ContextBoundary {
        run_id: "run-1".to_owned(),
        episode_id: "episode-1".to_owned(),
        agent_id: "agent-1".to_owned(),
        state_id: "combat-1".to_owned(),
        generation: 0,
        observation_sha256: "a".repeat(64),
        catalog_sha256: "b".repeat(64),
        adapter_revision: config.revision.clone(),
        model_revision: "model-reviewed-1".to_owned(),
        configuration_sha256: "c".repeat(64),
        output_schema_sha256: "d".repeat(64),
        controller_epoch: 1,
        gate_epoch: 0,
        control_version: 1,
    };
    let identity = ContextRenderSourceIdentity {
        owner_id: "owner-1".to_owned(),
        owner_version: "v1".to_owned(),
        catalog_digest: "e".repeat(64),
        binding_id: "binding-1".to_owned(),
        binding_version: 1,
        binding_digest: "f".repeat(64),
        invocation_id: "invocation-1".to_owned(),
        instance_id: "instance-1".to_owned(),
        lease_id: "lease-1".to_owned(),
        lease_epoch: 1,
        active_revision_id: "revision-1".to_owned(),
        source_id: "strategy-1".to_owned(),
        source_version: 1,
        source_digest: "9".repeat(64),
        membership_digest: None,
        boundary: boundary.clone(),
    };
    ContextRenderSource {
        source_id: "strategy-1".to_owned(),
        source_version: 1,
        source_digest: "9".repeat(64),
        active_revision_id: "revision-1".to_owned(),
        boundary,
        limits: ContextRenderLimits::harness_maxima(),
        document: ContextSourceDocument { draft, items },
        membership: None,
        continuity: MembershipContinuity::Stateless,
        now: 1,
        valid_until: 100,
        identity,
    }
}

#[test]
fn admitted_profile_reaches_actual_normal_and_prepared_exo_dispatch() {
    let mut source = source_with_profile(admitted_profile());
    let profile_ref = source.0.profiles[0].binding.profile_ref.clone();
    let input = input();

    assert_eq!(
        source.0.managed_render_config(),
        Some(source.2.clone()),
        "the admitted Exo source must expose its actual trusted render configuration"
    );
    assert_eq!(
        source
            .0
            .decide_for(&input, &profile_ref, LIVE_CONTEXT_REF)
            .expect("normal profile dispatch reaches Exo"),
        Decision::Action {
            action_id: "combat.end-turn".to_owned(),
            rationale: "normal profile decision".to_owned(),
            confidence: None,
        }
    );
    assert_eq!(source.1.load(Ordering::SeqCst), 1);

    let context = context_source(&source.2);
    let prepared = source
        .0
        .prepare_managed_context(&input, &context)
        .expect("the exact admitted config renders managed context");
    assert_eq!(
        source
            .0
            .decide_prepared_for(&input, &profile_ref, LIVE_CONTEXT_REF, &prepared)
            .expect("prepared profile dispatch reaches Exo"),
        Decision::Action {
            action_id: "combat.end-turn".to_owned(),
            rationale: "prepared profile decision".to_owned(),
            confidence: None,
        }
    );
    assert_eq!(source.1.load(Ordering::SeqCst), 2);
}

#[test]
fn stale_or_unsupported_binding_refuses_before_exo_transport() {
    let mut mismatched = admitted_profile();
    mismatched.descriptor.version = "1.0.1".to_owned();
    let mut source = source_with_profile(mismatched);
    let profile_ref = source.0.profiles[0].binding.profile_ref.clone();
    let input = input();

    assert_eq!(
        source.0.decide_for(&input, &profile_ref, LIVE_CONTEXT_REF),
        Err(PolicyError::InputBlocked)
    );
    assert_eq!(
        source
            .0
            .decide_for(&input, &profile_ref, "context.unsupported.v1"),
        Err(PolicyError::InputBlocked)
    );
    assert_eq!(source.1.load(Ordering::SeqCst), 0);
}
