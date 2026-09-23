// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_harness::{ActionKind, ActionSetError};

use super::super::super::runtime_v3_wire::action_kind_name;
use super::super::action_from_payload;
use super::{action_set, config, response};

fn catalog_with(action_id: &str, payload: Value) -> Value {
    let mut value = response("legal_actions_response", 0, Value::Null, Value::Null);
    value["observation"] = Value::Null;
    value["legal_actions"] = json!([{"action_id": action_id, "action": payload}]);
    value
}

fn continuation_catalog(payload: Value) -> Value {
    catalog_with("continue_run:2", payload)
}

#[test]
fn the_catalog_admits_host_offered_continuation_and_refuses_anything_else() -> Result<(), String> {
    for payload in [
        json!({"kind": "continue_run"}),
        json!({"kind": "continue_run", "run_id": "profile1"}),
    ] {
        let value = continuation_catalog(payload.clone());
        let (actions, payloads) = action_set(&value, "legal_actions_response", &config())?;
        let action = actions
            .find("continue_run:2")
            .ok_or("a host-offered continuation is missing from the catalog")?;
        assert_eq!(action.kind(), ActionKind::ContinueRun);
        assert_eq!(action_kind_name(action.kind()), "continue_run");
        assert_eq!(payloads["continue_run:2"], payload);
    }
    // The host owns the identity and the discriminator, so a caller cannot smuggle a save path, a
    // blank discriminator, a non-identity, or an extra field past the boundary.
    for (payload, reason) in [
        (
            json!({"kind": "continue_run", "save_path": "profile1/saves/current_run.save"}),
            "save path",
        ),
        (
            json!({"kind": "continue_run", "run_id": null}),
            "null run_id",
        ),
        (
            json!({"kind": "continue_run", "run_id": ""}),
            "empty run_id",
        ),
        (
            json!({"kind": "continue_run", "run_id": "profile 1"}),
            "non-identity run_id",
        ),
        (
            json!({"kind": "continue_run", "run_id": "profile1", "seed": "1"}),
            "extra field",
        ),
    ] {
        let value = continuation_catalog(payload);
        assert!(
            action_set(&value, "legal_actions_response", &config()).is_err(),
            "the catalog admitted a continuation carrying a {reason}"
        );
    }
    Ok(())
}

#[test]
fn a_continuation_is_bound_to_the_offered_generation_before_dispatch() -> Result<(), String> {
    let payload = json!({"kind": "continue_run", "run_id": "profile1"});
    let (actions, _) = action_set(
        &continuation_catalog(payload.clone()),
        "legal_actions_response",
        &config(),
    )?;
    // The episode runner asserts this pair before it dispatches, so a stale generation is refused
    // before the action can have an effect.
    assert!(actions.assert_matches("setup-1", 0).is_ok());
    assert_eq!(
        actions.assert_matches("setup-1", 1).err(),
        Some(ActionSetError::StaleObservation)
    );
    assert!(actions.find("continue_run:9").is_none());

    // Recording/replay rebuilds the action from the stored payload and keeps its identity.
    let replayed = action_from_payload("continue_run:2", &payload)?;
    assert_eq!(replayed.kind(), ActionKind::ContinueRun);
    assert_eq!(replayed.action_id(), "continue_run:2");
    assert!(action_from_payload("continue_run:2", &json!({"kind": "start_run"})).is_err());
    Ok(())
}

#[test]
fn a_malformed_continuation_is_refused_at_the_payload_boundary() -> Result<(), String> {
    // Every payload names the continuation but breaks one part of the agreed shape, so the
    // production parser refuses it instead of coercing the kind into another action.
    for (payload, reason) in [
        (json!("continue_run"), "payload that is not an object"),
        (json!({"run_id": "profile1"}), "payload without a kind"),
        (json!({"kind": 7}), "payload whose kind is not text"),
        (json!({"kind": "resume_run"}), "unowned kind"),
        (
            json!({"kind": "continue_run", "run_id": {"profile": "profile2"}}),
            "structured run_id",
        ),
        (
            json!({"kind": "continue_run", "profile": "profile2"}),
            "foreign profile field",
        ),
        (
            json!({"kind": "continue_run", "run_id": "profile2", "save_path": "a/save"}),
            "save path beside run_id",
        ),
    ] {
        let value = continuation_catalog(payload);
        assert!(
            action_set(&value, "legal_actions_response", &config()).is_err(),
            "the catalog admitted a continuation with a {reason}"
        );
    }

    // Recording/replay rebuilds from the stored payload through the same contract, so a stored
    // malformed continuation is refused on the way back in too.
    assert!(
        action_from_payload(
            "continue_run:2",
            &json!({"kind": "continue_run", "profile": "profile2"})
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn only_the_host_offered_continuation_is_dispatchable() -> Result<(), String> {
    // The host offers one profile's saved run and the catalog binds it to the offered generation.
    // A continuation for a different profile, or for a sequence the host never offered, has no
    // entry to find, so it cannot be dispatched.
    let offered = catalog_with(
        "continue_run:2:profile1",
        json!({"kind": "continue_run", "run_id": "profile1"}),
    );
    let (actions, _) = action_set(&offered, "legal_actions_response", &config())?;
    assert!(actions.find("continue_run:2:profile1").is_some());
    assert!(actions.find("continue_run:2:profile2").is_none());
    assert!(actions.find("continue_run:9:profile1").is_none());
    assert!(actions.assert_matches("setup-1", 0).is_ok());

    // A discriminator that names another profile's storage rather than an opaque host identity is
    // refused at the boundary: the harness never resolves a profile or a save location itself.
    for foreign in [
        "profile2\\saves\\current_run.save",
        "C:\\profiles\\profile2",
    ] {
        let value = catalog_with(
            "continue_run:2:profile2",
            json!({"kind": "continue_run", "run_id": foreign}),
        );
        assert!(
            action_set(&value, "legal_actions_response", &config()).is_err(),
            "the catalog admitted a foreign-profile discriminator {foreign}"
        );
    }
    Ok(())
}
