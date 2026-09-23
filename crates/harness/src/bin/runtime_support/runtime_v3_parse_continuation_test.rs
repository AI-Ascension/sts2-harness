// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_harness::{ActionKind, ActionSetError};

use super::super::super::runtime_v3_wire::action_kind_name;
use super::super::action_from_payload;
use super::{action_set, config, response};

fn continuation_catalog(payload: Value) -> Value {
    let mut value = response("legal_actions_response", 0, Value::Null, Value::Null);
    value["observation"] = Value::Null;
    value["legal_actions"] = json!([{"action_id": "continue_run:2", "action": payload}]);
    value
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
