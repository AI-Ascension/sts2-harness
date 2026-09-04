// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_harness::{ActionKind, DispatchStatus, EpisodeLegalAction};

use super::super::config::RuntimeConfig;
use super::{observation, receipt};

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
        "schema_digest": "fbfb18279b0c7ebb350ef0ce0d56547fa11e83985b13380cb2b0f1dba4cb56e9",
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
