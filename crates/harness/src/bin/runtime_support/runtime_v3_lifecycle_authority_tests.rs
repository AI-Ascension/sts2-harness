// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;
use serde_json::json;
use sts2_harness::exo_lifecycle::AuthorityVector;
use sts2_harness::{ActionKind, EpisodeLegalAction};

#[test]
fn provider_action_id_projection_has_its_own_internal_digest() {
    let ids = vec![String::from("combat.end-turn"), String::from("combat.play")];
    assert_eq!(
        catalog_digest_ids(&ids).expect("catalog digest"),
        sts2_harness::sha256_hex(
            serde_json::to_vec(&json!(["combat.end-turn", "combat.play"])).expect("encoded ids")
        )
    );
}

#[test]
fn same_action_ids_do_not_authorize_a_changed_owned_catalog() {
    let actions = EpisodeLegalActionSet::new(
        "combat-state",
        3,
        vec![
            EpisodeLegalAction::new("combat.end-turn", ActionKind::EndTurn).expect("legal action"),
        ],
    )
    .expect("legal actions");
    let catalog_v1 =
        br#"[{"action_id":"combat.end-turn","action":{"kind":"end_turn","target":"original"}}]"#;
    let catalog_v2 =
        br#"[{"action_id":"combat.end-turn","action":{"kind":"end_turn","target":"swapped"}}]"#;
    let scope = SessionScope::new("project", "run", "episode", "agent").expect("scope");
    let lineage =
        ExecutionLineage::new("run", "episode", "attempt", "trajectory").expect("lineage");
    let config_digest = sts2_harness::sha256_hex("config");
    let lease = LeaseAuthority {
        id: String::from("lease"),
        epoch: 1,
    };
    let turn = TurnAuthority {
        lease: lease.clone(),
        state_id: String::from("combat-state"),
        generation: 3,
        catalog_digest: catalog_digest(catalog_v1).expect("catalog digest"),
        action_ids_digest: action_ids_digest(&actions).expect("action ID digest"),
    };
    let manifest = InvocationManifest {
        scope: scope.clone(),
        execution_id: String::from("execution"),
        episode_attempt_id: lineage.attempt_id.clone(),
        trajectory_id: lineage.trajectory_id.clone(),
        provider_attempt_id: String::from("provider-attempt"),
        reservation_id: String::from("reservation"),
        binding_id: String::from("binding"),
        operation_id: String::from("operation"),
        prepared_id: String::from("prepared"),
        request_id: String::from("request"),
        host_turn_id: String::from("turn"),
        input_digest: sts2_harness::sha256_hex("input"),
        input_length: 1,
        config_digest: config_digest.clone(),
        package_digest: sts2_harness::sha256_hex("package"),
        profile_digest: sts2_harness::sha256_hex("profile"),
        model_revision: String::from("model"),
        reserved_units: 1,
        authority: AuthorityVector {
            owner_epoch: 1,
            auth_epoch: 1,
            session_epoch: 1,
            history_epoch: 0,
            compaction_epoch: 0,
            revocation_epoch: 0,
            lease_id: lease.id.clone(),
            lease_epoch: lease.epoch,
            state_id: turn.state_id.clone(),
            generation: turn.generation,
            catalog_digest: turn.catalog_digest.clone(),
        },
    };
    let state = RuntimeLifecycleAuthorityState::default();
    *state.0.lock().expect("runtime authority lock") = AuthoritySnapshot {
        enabled: true,
        fence: None,
        lease: Some(lease),
        turn: Some(turn),
    };
    assert!(
        state
            .lock_for_manifest(&scope, &lineage, &config_digest, &manifest, None, None)
            .is_ok()
    );
    state
        .update_catalog(&actions, catalog_v2)
        .expect("same-ID catalog update");
    assert!(matches!(
        state.lock_for_manifest(&scope, &lineage, &config_digest, &manifest, None, None),
        Err(LifecycleError::Stale)
    ));
}
