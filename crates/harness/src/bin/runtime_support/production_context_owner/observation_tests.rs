// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::super::*;
use sts2_harness::management::MANAGEMENT_SCHEMA_VERSION;
use sts2_harness::management::{AuthContext, ContextBindingRequest, ContextControlCommand};
use sts2_harness::{ActionKind, EpisodeLegalAction, EpisodeStage};

fn setup() -> (
    Owner,
    AuthContext,
    RunRequest,
    RuntimeAuthorityBinding,
    String,
    ContextOwnerControlLimits,
) {
    let directory =
        std::env::temp_dir().join(format!("context-owner-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("directory");
    let configuration = Configuration {
        schema_version: SCHEMA.into(),
        store_path: directory.join("context.sqlite3"),
        key_reference: "unused".into(),
        owner_id: "owner".into(),
        owner_version: "1".into(),
        context_ref: "context.live".into(),
        limits: ContextEffectiveLimits {
            max_items: 1,
            max_notes: 1,
            max_context_bytes: 1,
            max_objective_bytes: 1,
            max_control_events: 64,
        },
    };
    let request = RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.into(),
        request_id: "request".into(),
        definition: None,
        artifact_id: None,
        instance_id: "instance".into(),
        profile: "live.workflow.v1".into(),
        admission: None,
    };
    let digest = "a".repeat(64);
    let binding = RuntimeAuthorityBinding {
        instance_id: request.instance_id.clone(),
        session_id: "session".into(),
        lease_id: "lease".into(),
        lease_epoch: 1,
        run_id: run_id(&request, &digest).expect("run"),
        episode_id: "episode".into(),
        trajectory_id: "trajectory".into(),
        trace_id: "trace".into(),
        artifact_id: "artifact".into(),
        agent_id: "agent".into(),
        adapter_revision: "adapter".into(),
        model_revision: "model".into(),
        configuration_digest: "b".repeat(64),
        output_schema_digest: "c".repeat(64),
    };
    let actor = AuthContext::new("actor", ["workflow:control".into()]).expect("actor");
    let owner = Owner {
        configuration,
        key: [7; 32],
        current: Mutex::new(BTreeMap::new()),
    };
    let catalog = owner.catalog(&actor).expect("catalog");
    let selected = ContextOwnerControlLimits::from_descriptors(
        &catalog,
        catalog.descriptors.iter().collect::<Vec<_>>().as_slice(),
    )
    .expect("selected control limits");
    (owner, actor, request, binding, digest, selected)
}

fn observation(generation: u64) -> EpisodeObservation {
    EpisodeObservation::new("combat-1", generation, EpisodeStage::Combat, true, false, true, serde_json::json!({
        "state_id":"combat-1", "generation":generation, "visible_seed":"seed",
        "player":{"hp":1,"max_hp":1,"energy":1,"gold":0,"hand":[],"deck":[],"discard":[],"exhaust":[]},
        "state":{"state":"combat","turn_index":1,"enemies":[]},
        "legal_actions":[{"action_id":"combat.end-turn","action":{"kind":"end_turn"}}]
    })).expect("observation")
}

#[test]
fn stale_runtime_lease_cannot_replace_current_observation_before_effects() {
    let (owner, actor, request, binding, digest, selected) = setup();
    owner
        .record_observation(
            &actor,
            &request,
            &digest,
            &binding,
            &observation(1),
            &selected,
        )
        .expect("first");
    for stale in [
        RuntimeAuthorityBinding {
            lease_id: "replacement-lease".into(),
            ..binding.clone()
        },
        RuntimeAuthorityBinding {
            lease_epoch: 2,
            ..binding.clone()
        },
    ] {
        let error = owner
            .record_observation(
                &actor,
                &request,
                &digest,
                &stale,
                &observation(2),
                &selected,
            )
            .expect_err("stale lease");
        assert_eq!(error.code, "context_owner_runtime_scope");
    }
    let current = owner.current.lock().expect("lock");
    assert_eq!(
        current[&binding.run_id]
            .authority
            .state()
            .boundary
            .generation,
        1
    );
}

#[test]
fn stale_catalog_generation_cannot_make_current_boundary_bindable() {
    let (owner, actor, request, binding, digest, selected) = setup();
    owner
        .record_observation(
            &actor,
            &request,
            &digest,
            &binding,
            &observation(1),
            &selected,
        )
        .expect("observation");
    let actions = EpisodeLegalActionSet::new(
        "combat-1",
        2,
        vec![EpisodeLegalAction::new("combat.end-turn", ActionKind::EndTurn).expect("action")],
    )
    .expect("actions");
    let error = owner
        .record_legal_actions(&actor, &request, &digest, &binding, &actions)
        .expect_err("stale");
    assert_eq!(error.code, "context_owner_catalog_stale");
    let current = owner.current.lock().expect("lock");
    assert_eq!(current[&binding.run_id].catalog_generation, None);
}

#[test]
fn recovered_authority_drops_the_prior_catalog_at_the_new_observation_boundary() {
    let (owner, actor, request, binding, digest, selected) = setup();
    owner
        .record_observation(
            &actor,
            &request,
            &digest,
            &binding,
            &observation(1),
            &selected,
        )
        .expect("observation");
    let actions = EpisodeLegalActionSet::new(
        "combat-1",
        1,
        vec![EpisodeLegalAction::new("combat.end-turn", ActionKind::EndTurn).expect("action")],
    )
    .expect("actions");
    owner
        .record_legal_actions(&actor, &request, &digest, &binding, &actions)
        .expect("catalog");
    owner.current.lock().expect("lock").clear();
    owner
        .record_observation(
            &actor,
            &request,
            &digest,
            &binding,
            &observation(2),
            &selected,
        )
        .expect("recovered observation");
    let current = owner.current.lock().expect("lock");
    let entry = &current[&binding.run_id];
    assert_eq!(entry.authority.state().boundary.generation, 2);
    assert_eq!(entry.catalog_generation, None);
}

#[test]
fn served_owner_control_refuses_commit_past_the_admitted_run_limit() {
    let (owner, actor, request, runtime_binding, digest, selected) = setup();
    let selected = ContextOwnerControlLimits {
        max_control_events: 1,
        ..selected
    };
    owner
        .record_observation(
            &actor,
            &request,
            &digest,
            &runtime_binding,
            &observation(1),
            &selected,
        )
        .expect("admitted observation");
    let actions = EpisodeLegalActionSet::new(
        "combat-1",
        1,
        vec![EpisodeLegalAction::new("combat.end-turn", ActionKind::EndTurn).expect("action")],
    )
    .expect("actions");
    owner
        .record_legal_actions(&actor, &request, &digest, &runtime_binding, &actions)
        .expect("legal actions");

    let catalog = owner.catalog(&actor).expect("current catalog");
    let descriptor = &catalog.descriptors[0];
    let binding_request = ContextBindingRequest {
        workflow_run_id: runtime_binding.run_id.clone(),
        definition_digest: digest,
        instance_id: request.instance_id.clone(),
        graph_id: "graph-1".into(),
        node_id: "node-1".into(),
        node_execution_id: "execution-1".into(),
        node_kind: "decide".into(),
        context_ref: descriptor.context_ref.clone(),
        binding_id: descriptor.binding_id.clone(),
        binding_version: descriptor.version,
        binding_digest: descriptor.digest.clone(),
    };
    let binding = owner
        .bind(&actor, &binding_request)
        .expect("current binding");
    let mut harness_max_authority = ControlAuthority::new(
        binding.boundary.clone(),
        binding.approved_revision_id.clone(),
    );
    let baseline_pause = harness_max_authority
        .request_pause("baseline-pause", binding.boundary.control_version)
        .expect("harness maximum accepts pause");
    let baseline_boundary = harness_max_authority.state().boundary.clone();
    harness_max_authority
        .commit(
            "baseline-commit",
            baseline_pause.control_version,
            &binding.approved_revision_id,
            &baseline_boundary,
            &"e".repeat(64),
            &"e".repeat(64),
        )
        .expect("harness maximum accepts commit");

    owner
        .control(
            &actor,
            &binding,
            &ContextControlCommand::Pause {
                idempotency_key: "selected-pause".into(),
                expected_control_version: binding.boundary.control_version,
            },
        )
        .expect("selected authority accepts first control event");
    let paused_binding = owner
        .bind(&actor, &binding_request)
        .expect("binding reflects paused boundary");
    let error = owner
        .control(
            &actor,
            &paused_binding,
            &ContextControlCommand::Commit {
                idempotency_key: "selected-commit".into(),
                expected_control_version: paused_binding.boundary.control_version,
                expected_revision_id: paused_binding.approved_revision_id.clone(),
                expected_boundary: paused_binding.boundary.clone(),
                preview_manifest_digest: "e".repeat(64),
                approved_manifest_digest: "e".repeat(64),
            },
        )
        .expect_err("selected cap of one event must refuse two-event commit");
    assert_eq!(error.code, "context_control_events_exhausted");
}

#[path = "association_receipt_tests.rs"]
mod association_receipt_tests;
