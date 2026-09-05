// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_harness::{ActionKind, DispatchStatus, EpisodeLegalAction};

use super::super::config::RuntimeConfig;
use super::{action_set, observation, receipt, result_observation, wait_sample};

#[path = "runtime_v3_contract_test.rs"]
mod contract;

#[test]
fn unknown_results_still_validate_generation_and_nullable_state_identity() -> Result<(), String> {
    let action = EpisodeLegalAction::new("combat.end-turn", ActionKind::EndTurn)
        .map_err(|error| error.to_string())?;
    for (field, invalid) in [
        ("generation", json!("bad")),
        ("generation", json!(9_007_199_254_740_992_u64)),
        ("state_id", json!({})),
        ("state_id", json!([])),
        ("state_id", json!("")),
    ] {
        let mut value = response(
            "dispatch_action_response",
            0,
            json!("op-1"),
            json!("unknown"),
        );
        value["observation"] = Value::Null;
        value["legal_actions"] = Value::Null;
        value["error_code"] = json!("host_pending");
        value["state_id"] = Value::Null;
        receipt(
            &value,
            "dispatch_action_response",
            &config(),
            "op-1",
            0,
            action.clone(),
        )?;
        value[field] = invalid;
        assert!(
            receipt(
                &value,
                "dispatch_action_response",
                &config(),
                "op-1",
                0,
                action.clone()
            )
            .is_err()
        );
        value["kind"] = json!("wait_response");
        value["wait_outcome"] = json!("timeout");
        assert!(wait_sample(&value, &config(), "op-1", 0).is_err());
    }
    Ok(())
}

fn config() -> RuntimeConfig {
    RuntimeConfig {
        gateway_address: String::from("127.0.0.1:15525"),
        gateway_token: String::from("test-token"),
        mcp_binary: String::from("mcp"),
        runtime_profile: String::from("runtime-v3-gameplay"),
        instance_id: String::from("instance-1"),
        caller_id: String::from("harness"),
        session_id: String::from("session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        mcp_session_id: String::from("mcp-session-1"),
        run_id: String::from("run-1"),
        episode_id: String::from("episode-1"),
        trajectory_id: String::from("trajectory-1"),
        artifact_id: String::from("artifact-1"),
        wait_for_combat_seconds: 0,
        settlement_timeout_seconds: 0,
    }
}

fn response(kind: &str, generation: u64, operation_id: Value, status: Value) -> Value {
    let combat = generation > 0;
    let state_id = if combat { "combat-1" } else { "setup-1" };
    let state = if combat {
        json!({"state": "combat", "turn_index": 1, "enemies": []})
    } else {
        json!({"state": "setup", "characters": ["ironclad"]})
    };
    let actions = if combat {
        json!([{"action_id": "combat.end-turn", "action": {"kind": "end_turn"}}])
    } else {
        json!([{"action_id": "run.start", "action": {"kind": "start_run", "character_id": "ironclad"}}])
    };
    let transition = if combat {
        json!({
            "from_generation": 0,
            "to_generation": 1,
            "state_id": state_id,
            "effect_kind": "end_turn.settled"
        })
    } else {
        Value::Null
    };
    json!({
        "protocol_version": "runtime-v3-gameplay",
        "schema_digest": "b37c80f583aeaf4f81ede2083bcfb4129196baf5eb092470e8738173c4b7226c",
        "provenance": {"artifact": "sts2-protocol/runtime-v3-gameplay", "source": "schemas/runtime-v3-gameplay.schema.json", "generator": "hand-authored"},
        "correlation_id": "7",
        "instance_id": "instance-1",
        "session_id": "session-1",
        "lease_id": "lease-1",
        "lease_epoch": 1,
        "generation": generation,
        "kind": kind,
        "state_id": state_id,
        "operation_id": operation_id,
        "observation": {"state_id": state_id, "generation": generation, "visible_seed": null, "player": {"hp": 50, "max_hp": 50, "energy": 3, "gold": 99, "hand": [], "deck": [], "discard": [], "exhaust": []}, "state": state},
        "legal_actions": actions,
        "action": null,
        "status": status,
        "transition": transition,
        "error_code": null,
        "wait_for_millis": null,
        "wait_outcome": null,
        "recovery": null
    })
}

#[test]
fn observation_separates_semantic_action_from_host_payload() -> Result<(), String> {
    let parsed = observation(
        &response("state_response", 0, Value::Null, Value::Null),
        "state_response",
        &config(),
    )?;
    assert_eq!(parsed.actions.actions()[0].kind(), ActionKind::StartRun);
    assert_eq!(parsed.payloads["run.start"]["kind"], "start_run");
    assert_eq!(parsed.observation.generation(), 0);
    Ok(())
}

#[test]
fn privileged_projection_is_rejected_before_policy_use() {
    let mut value = response("state_response", 0, Value::Null, Value::Null);
    value["observation"]["rng_state"] = json!("hidden");
    assert!(observation(&value, "state_response", &config()).is_err());
}

#[test]
fn settled_receipt_requires_and_preserves_a_fresh_witness() -> Result<(), String> {
    let action = EpisodeLegalAction::new("combat.end-turn", ActionKind::EndTurn)
        .map_err(|error| error.to_string())?;
    let receipt = receipt(
        &response(
            "dispatch_action_response",
            1,
            json!("op-1"),
            json!("settled"),
        ),
        "dispatch_action_response",
        &config(),
        "op-1",
        0,
        action,
    )?;
    assert_eq!(receipt.status(), DispatchStatus::Settled);
    assert_eq!(receipt.after().map(|value| value.generation()), Some(1));
    assert_eq!(receipt.effect_kind(), Some("end_turn.settled"));
    Ok(())
}

#[test]
fn settlement_must_start_at_the_operations_original_generation() -> Result<(), String> {
    let action = EpisodeLegalAction::new("combat.end-turn", ActionKind::EndTurn)
        .map_err(|error| error.to_string())?;
    let mut value = response(
        "dispatch_action_response",
        3,
        json!("op-1"),
        json!("settled"),
    );
    value["transition"]["to_generation"] = json!(3);
    value["transition"]["from_generation"] = json!(2);
    assert!(
        receipt(
            &value,
            "dispatch_action_response",
            &config(),
            "op-1",
            0,
            action.clone()
        )
        .is_err()
    );
    value["transition"]["from_generation"] = json!(0);
    assert!(
        receipt(
            &value,
            "dispatch_action_response",
            &config(),
            "op-1",
            0,
            action
        )
        .is_ok()
    );
    value["kind"] = json!("wait_response");
    value["wait_outcome"] = json!("successor");
    // Even after observation advances, waits bind to the ledger's original operation generation.
    assert!(wait_sample(&value, &config(), "op-1", 0).is_ok());
    value["transition"]["from_generation"] = json!(2);
    assert!(wait_sample(&value, &config(), "op-1", 0).is_err());
    Ok(())
}

#[test]
fn response_shapes_reject_request_payloads_and_contradictory_errors() -> Result<(), String> {
    let action = EpisodeLegalAction::new("combat.end-turn", ActionKind::EndTurn)
        .map_err(|error| error.to_string())?;
    for (field, invalid) in [
        ("action", json!({"kind": "save_quit"})),
        ("wait_for_millis", json!(10)),
        ("recovery", json!({"kind": "reobserve"})),
        ("wait_outcome", json!("successor")),
        ("error_code", json!("contradiction")),
    ] {
        let mut value = response(
            "dispatch_action_response",
            1,
            json!("op-1"),
            json!("settled"),
        );
        value[field] = invalid;
        assert!(
            receipt(
                &value,
                "dispatch_action_response",
                &config(),
                "op-1",
                0,
                action.clone()
            )
            .is_err(),
            "{field}"
        );
    }
    let mut value = response("wait_response", 0, json!("op-1"), json!("unknown"));
    value["observation"] = Value::Null;
    value["legal_actions"] = Value::Null;
    value["wait_outcome"] = json!("timeout");
    assert!(wait_sample(&value, &config(), "op-1", 0).is_err());
    value["error_code"] = json!("host_pending");
    assert!(wait_sample(&value, &config(), "op-1", 0).is_ok());
    for invalid in [json!(""), json!(17), json!("private arbitrary text")] {
        value["error_code"] = invalid;
        assert!(wait_sample(&value, &config(), "op-1", 0).is_err());
    }
    let mut state = response("state_response", 0, Value::Null, Value::Null);
    state["action"] = json!({"kind": "end_turn"});
    assert!(observation(&state, "state_response", &config()).is_err());
    Ok(())
}

#[test]
fn validated_results_install_their_observation_without_using_state_response_shape()
-> Result<(), String> {
    let action = EpisodeLegalAction::new("combat.end-turn", ActionKind::EndTurn)
        .map_err(|error| error.to_string())?;
    for kind in [
        "dispatch_action_response",
        "recover_response",
        "wait_response",
    ] {
        let mut value = response(kind, 1, json!("op-1"), json!("settled"));
        if kind == "wait_response" {
            value["wait_outcome"] = json!("successor");
            wait_sample(&value, &config(), "op-1", 0)?;
        } else {
            receipt(&value, kind, &config(), "op-1", 0, action.clone())?;
        }
        let parsed = result_observation(&value, kind, &config())?;
        assert_eq!(parsed.observation.generation(), 1);
        assert!(parsed.actions.find("combat.end-turn").is_some());
        assert_eq!(parsed.payloads["combat.end-turn"]["kind"], "end_turn");
        value["action"] = json!({"kind": "end_turn"});
        assert!(result_observation(&value, kind, &config()).is_err());
    }
    let mut legal = response("legal_actions_response", 0, Value::Null, Value::Null);
    assert!(action_set(&legal, "legal_actions_response", &config()).is_err());
    legal["observation"] = Value::Null;
    assert!(action_set(&legal, "legal_actions_response", &config()).is_ok());
    Ok(())
}
