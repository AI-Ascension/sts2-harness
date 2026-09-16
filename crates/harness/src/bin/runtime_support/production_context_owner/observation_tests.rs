// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::super::*;
use sts2_harness::management::MANAGEMENT_SCHEMA_VERSION;
use sts2_harness::{ActionKind, EpisodeLegalAction, EpisodeStage};

fn setup() -> (
    Owner,
    AuthContext,
    RunRequest,
    RuntimeAuthorityBinding,
    String,
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
            max_control_events: 8,
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
    (
        Owner {
            configuration,
            key: [7; 32],
            current: Mutex::new(BTreeMap::new()),
        },
        actor,
        request,
        binding,
        digest,
    )
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
    let (owner, actor, request, binding, digest) = setup();
    owner
        .record_observation(&actor, &request, &digest, &binding, &observation(1))
        .expect("first");
    let mut stale = binding.clone();
    stale.lease_epoch = 2;
    let error = owner
        .record_observation(&actor, &request, &digest, &stale, &observation(2))
        .expect_err("stale");
    assert_eq!(error.code, "context_owner_runtime_scope");
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
    let (owner, actor, request, binding, digest) = setup();
    owner
        .record_observation(&actor, &request, &digest, &binding, &observation(1))
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
    let (owner, actor, request, binding, digest) = setup();
    owner
        .record_observation(&actor, &request, &digest, &binding, &observation(1))
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
        .record_observation(&actor, &request, &digest, &binding, &observation(2))
        .expect("recovered observation");
    let current = owner.current.lock().expect("lock");
    let entry = &current[&binding.run_id];
    assert_eq!(entry.authority.state().boundary.generation, 2);
    assert_eq!(entry.catalog_generation, None);
}
