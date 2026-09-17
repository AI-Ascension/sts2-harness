// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;
use std::collections::BTreeMap;
use std::fs;
use sts2_harness::context_control::{
    ContextDraft, ContextItem, ContextItemRef, ContextSourceDocument, ControlAuthority, StoreMode,
    context_source_digest,
};
use sts2_harness::management::{
    Budget, CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION, CleanupState, ContextBindingRequest,
    ContextSourceAdoptionRequest, Cursor, GameOutcome, RUN_SCHEMA_VERSION, RunSnapshot,
    WorkflowRunStatus,
};
use sts2_harness::{
    ActionKind, DecisionInput, EpisodeLegalAction, EpisodeLegalActionSet, EpisodeObservation,
    EpisodeStage, ModelExecutionId,
};

fn fixture_input() -> (
    EpisodeObservation,
    EpisodeLegalActionSet,
    ContextSourceDocument,
) {
    let observation = EpisodeObservation::new(
        "combat-1",
        1,
        EpisodeStage::Combat,
        true,
        false,
        true,
        serde_json::json!({
            "state_id":"combat-1",
            "generation":1,
            "visible_seed":"fixture",
            "player":{"hp":10,"max_hp":10,"energy":3,"gold":0,"hand":[],"deck":[],"discard":[],"exhaust":[]},
            "state":{"state":"combat","turn_index":1,"enemies":[]},
            "legal_actions":[{"action_id":"combat.end-turn","action":{"kind":"end_turn"}}]
        }),
    )
    .expect("observation");
    let actions = EpisodeLegalActionSet::new(
        "combat-1",
        1,
        vec![
            EpisodeLegalAction::new("combat.end-turn", ActionKind::EndTurn).expect("legal action"),
        ],
    )
    .expect("legal action set");
    let mut items = BTreeMap::new();
    let mut draft = ContextDraft::new("draft.1", "context.revision.1");
    for (item_id, bytes) in [
        ("strategy.1", b"trusted first item".to_vec()),
        ("strategy.2", b"trusted second item".to_vec()),
    ] {
        let item = ContextItem {
            reference: ContextItemRef {
                item_id: item_id.to_owned(),
                version: 1,
                sha256: sts2_harness::sha256_hex(&bytes),
            },
            kind: "strategy".to_owned(),
            bytes,
            protected: false,
            expires_at: u64::MAX,
        };
        draft.selected_items.push(item.reference.clone());
        items.insert(format!("{item_id}:1"), item);
    }
    (observation, actions, ContextSourceDocument { draft, items })
}

#[test]
fn production_owner_activates_adopted_source_and_returns_descriptor_selected_limits() {
    let path = std::env::temp_dir().join(format!(
        "production-context-source-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&path).expect("create test store directory");
    let (observation, actions, document) = fixture_input();
    let definition_digest = "d".repeat(64);
    let source_digest = context_source_digest(&document).expect("source digest");
    let source = ContextBindingSource {
        source_id: "strategy".to_owned(),
        version: 1,
        digest: source_digest,
    };
    let limits = ContextEffectiveLimits {
        max_items: 1,
        ..ContextEffectiveLimits::default()
    };
    let configuration = Configuration {
        schema_version: SCHEMA.to_owned(),
        store_path: path.join("context.sqlite3"),
        key_reference: "unused-in-test".to_owned(),
        owner_id: "production-owner".to_owned(),
        owner_version: "v1".to_owned(),
        context_ref: "context.live.v1".to_owned(),
        limits,
        render_required: true,
        sources: vec![source.clone()],
    };
    let owner = Owner {
        configuration,
        key: [0x47; 32],
        current: Mutex::new(BTreeMap::new()),
    };
    let actor =
        AuthContext::new("render-operator", ["workflow:*".to_owned()]).expect("authorized actor");
    let request = RunRequest {
        schema_version: sts2_harness::management::MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: "render-source-run".to_owned(),
        definition: None,
        artifact_id: None,
        instance_id: "test-instance".to_owned(),
        profile: sts2_harness::management::LIVE_WORKFLOW_PROFILE.to_owned(),
        admission: None,
    };
    let run_id = run_id(&request, &definition_digest).expect("run id");
    let catalog = owner.catalog(&actor).expect("owner catalog");
    let descriptor = catalog.descriptors.first().expect("decide descriptor");
    let control_limits = ContextOwnerControlLimits::from_descriptors(&catalog, &[descriptor])
        .expect("admitted limits");
    let runtime_binding = RuntimeAuthorityBinding {
        instance_id: request.instance_id.clone(),
        session_id: "session-1".to_owned(),
        lease_id: "gateway-lease-1".to_owned(),
        lease_epoch: 3,
        run_id: run_id.clone(),
        episode_id: "episode-1".to_owned(),
        trajectory_id: "trajectory-1".to_owned(),
        trace_id: "trace-1".to_owned(),
        artifact_id: "artifact-1".to_owned(),
        agent_id: "agent-1".to_owned(),
        adapter_revision: "adapter-1".to_owned(),
        model_revision: "model-1".to_owned(),
        configuration_digest: "c".repeat(64),
        output_schema_digest: "e".repeat(64),
    };
    let observation_digest = sts2_harness::sha256_hex(
        serde_json::to_vec(observation.fair_play().as_value()).expect("observation bytes"),
    );
    let boundary = sts2_harness::context_control::ContextBoundary {
        run_id: run_id.clone(),
        episode_id: runtime_binding.episode_id.clone(),
        agent_id: runtime_binding.agent_id.clone(),
        state_id: observation.state_id().to_owned(),
        generation: observation.generation(),
        observation_sha256: observation_digest,
        catalog_sha256: legal_catalog_digest(&actions).expect("catalog digest"),
        adapter_revision: runtime_binding.adapter_revision.clone(),
        model_revision: runtime_binding.model_revision.clone(),
        configuration_sha256: runtime_binding.configuration_digest.clone(),
        output_schema_sha256: runtime_binding.output_schema_digest.clone(),
        controller_epoch: 1,
        gate_epoch: 1,
        control_version: 1,
    };
    let authority = ControlAuthority::new(boundary.clone(), "context.revision.1")
        .with_max_control_events(control_limits.max_control_events)
        .expect("selected event bound");
    let store_path = scoped_store_path(&owner.configuration.store_path, &run_id);
    let store = ContextControlStore::create(
        &store_path,
        owner.key,
        &run_id,
        &authority,
        StoreMode::Enabled,
    )
    .expect("create encrypted owner store");
    owner.current.lock().expect("owner lock").insert(
        run_id.clone(),
        Current {
            authority,
            store,
            actor: actor.subject.clone(),
            definition_digest: definition_digest.clone(),
            binding_request: None,
            catalog_generation: Some(observation.generation()),
            runtime_instance_id: runtime_binding.instance_id.clone(),
            runtime_lease_id: runtime_binding.lease_id.clone(),
            runtime_lease_epoch: runtime_binding.lease_epoch,
            admitted_control_limits: control_limits.clone(),
        },
    );

    let bind_request = ContextBindingRequest {
        workflow_run_id: run_id.clone(),
        definition_digest: definition_digest.clone(),
        instance_id: request.instance_id.clone(),
        graph_id: "graph.1".to_owned(),
        node_id: "node.1".to_owned(),
        node_execution_id: "node-execution.1".to_owned(),
        node_kind: "decide".to_owned(),
        context_ref: owner.configuration.context_ref.clone(),
        binding_id: descriptor.binding_id.clone(),
        binding_version: descriptor.version,
        binding_digest: descriptor.digest.clone(),
    };
    owner
        .bind(&actor, &bind_request)
        .expect("bind current production owner");
    let snapshot = RunSnapshot {
        schema_version: RUN_SCHEMA_VERSION.to_owned(),
        workflow_run_id: run_id.clone(),
        definition_digest: definition_digest.clone(),
        run_revision: 1,
        status: WorkflowRunStatus::Running,
        game_outcome: GameOutcome::NotTerminal,
        cursor: Cursor {
            graph_id: bind_request.graph_id.clone(),
            node_id: bind_request.node_id.clone(),
            node_execution_id: bind_request.node_execution_id.clone(),
        },
        pending_operation: None,
        budget: Budget::default(),
        cleanup: CleanupState::NotStarted,
        admission: None,
        execution_mode: None,
    };
    let before_adoption = owner
        .source_status_current(&actor, &snapshot)
        .expect("source status before adoption");
    assert!(before_adoption.active_source.is_none());
    owner
        .publish_source_current(&actor, &snapshot, &source.source_id, &document)
        .expect("publish allowlisted source");
    owner
        .adopt_source_current(
            &actor,
            &snapshot,
            &source.source_id,
            &ContextSourceAdoptionRequest {
                schema_version: CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION.to_owned(),
                idempotency_key: "adopt-source.1".to_owned(),
                expected_control_version: before_adoption.boundary.control_version,
                expected_revision_id: before_adoption.active_revision_id.clone(),
                expected_boundary: before_adoption.boundary.clone(),
            },
        )
        .expect("explicitly adopt source");

    let input = DecisionInput::new(
        ModelExecutionId::new(1).expect("execution id"),
        observation,
        actions,
        "survive",
        Vec::new(),
    );
    let rendered = owner
        .render_source_for_decision(
            &actor,
            &request,
            &definition_digest,
            &runtime_binding,
            &control_limits,
            &input,
            &owner.configuration.context_ref,
        )
        .expect("resolve active production source");

    assert_eq!(rendered.limits.max_items, 1);
    assert_eq!(rendered.document.draft.selected_items.len(), 2);
    assert_eq!(
        rendered.active_revision_id,
        owner
            .source_status_current(&actor, &snapshot)
            .expect("source status after adoption")
            .active_revision_id
    );
    let _ = fs::remove_dir_all(path);
}
