// SPDX-License-Identifier: MIT

use std::{fs, path::PathBuf};

use serde_json::{Value, json};
use sts2_harness::{ActionKind, DispatchStatus, EpisodeLegalAction};

fn artifact() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../protocol-artifact/runtime-v3-gameplay")
}

fn read_json(path: &str) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_slice(&fs::read(artifact().join(path))?)?)
}

#[test]
fn copied_contract_matches_authoritative_byte_inventory() -> Result<(), Box<dyn std::error::Error>>
{
    let sums = fs::read(artifact().join("SHA256SUMS"))?;
    assert_eq!(
        sts2_harness::sha256_hex(&sums),
        "ddc7c0a3697bcb474de8e7967041302dab072e11bc9990ffd5a508eb391cc1db"
    );
    let sums = String::from_utf8(sums)?;
    assert_eq!(sums.lines().count(), 11);
    for line in sums.lines() {
        let (digest, upstream_path) = line.split_once("  ").ok_or("invalid checksum record")?;
        // Preserve canonical relative references to the source and conformance mirrors.
        let local = match upstream_path {
            "../../schemas/runtime-v3-gameplay.schema.json"
            | "../../conformance/cases/runtime-v3-gameplay.json" => upstream_path,
            path if !path.contains("..") => path,
            _ => return Err("unexpected upstream path".into()),
        };
        assert_eq!(
            sts2_harness::sha256_hex(fs::read(artifact().join(local))?),
            digest
        );
    }
    assert_eq!(
        sts2_harness::sha256_hex(fs::read(artifact().join("schema.json"))?),
        super::super::SCHEMA_DIGEST
    );
    Ok(())
}

#[test]
fn canonical_goldens_validate_and_reach_actual_consumer_parser()
-> Result<(), Box<dyn std::error::Error>> {
    let schema = read_json("schema.json")?;
    let validator = jsonschema::validator_for(&schema)?;
    for file in [
        "state-request.json",
        "state-response.json",
        "dispatch-action-request.json",
        "dispatch-action-settled.json",
        "dispatch-proceed-request.json",
        "dispatch-confirm-selection-request.json",
        "dispatch-cancel-selection-request.json",
    ] {
        let value = read_json(&format!("golden/{file}"))?;
        assert!(validator.is_valid(&value), "{file}");
    }
    let state = read_json("golden/state-response.json")?;
    let parsed = super::observation(&state, "state_response", &super::config())?;
    assert_eq!(parsed.observation.generation(), 0);
    assert_eq!(parsed.observation.state_id(), "combat-1");
    let settled = read_json("golden/dispatch-action-settled.json")?;
    let action = EpisodeLegalAction::new("combat.end-turn", ActionKind::EndTurn)?;
    let receipt = super::receipt(
        &settled,
        &settled.to_string(),
        "dispatch_action_response",
        &super::config(),
        "op-1",
        0,
        action,
    )?;
    assert_eq!(receipt.status(), DispatchStatus::Settled);
    assert_eq!(receipt.after().map(|after| after.generation()), Some(1));
    assert_eq!(receipt.effect_kind(), Some("combat.end-turn_settled"));
    Ok(())
}

#[test]
fn continuation_goldens_reach_policy_without_accepting_extra_arguments()
-> Result<(), Box<dyn std::error::Error>> {
    for (file, kind) in [
        ("dispatch-proceed-request.json", ActionKind::Proceed),
        (
            "dispatch-confirm-selection-request.json",
            ActionKind::ConfirmSelection,
        ),
        (
            "dispatch-cancel-selection-request.json",
            ActionKind::CancelSelection,
        ),
    ] {
        let request = read_json(&format!("golden/{file}"))?;
        let mut state = read_json("golden/state-response.json")?;
        state["legal_actions"] = json!([{
            "action_id": "continuation-1",
            "action": request["action"]["action"].clone()
        }]);
        let parsed = super::observation(&state, "state_response", &super::config())?;
        assert_eq!(parsed.actions.actions()[0].kind(), kind);
        assert_eq!(
            parsed.payloads["continuation-1"],
            request["action"]["action"]
        );
        state["legal_actions"][0]["action"]["choice_id"] = json!("injected");
        assert!(super::observation(&state, "state_response", &super::config()).is_err());
    }
    Ok(())
}

#[test]
fn canonical_response_mutations_fail_at_actual_consumer_boundary()
-> Result<(), Box<dyn std::error::Error>> {
    let validator = jsonschema::validator_for(&read_json("schema.json")?)?;
    let source = read_json("golden/state-response.json")?;
    for (field, invalid) in [
        ("generation", json!(9_007_199_254_740_992_u64)),
        ("action", json!({"kind": "end_turn"})),
        ("correlation_id", json!("")),
        ("provenance", json!({})),
    ] {
        let mut value = source.clone();
        value[field] = invalid;
        assert!(!validator.is_valid(&value), "schema accepted {field}");
        assert!(
            super::observation(&value, "state_response", &super::config()).is_err(),
            "{field}"
        );
    }
    // Schema digests are consumer provenance pins, not self-referential schema constants.
    let mut stale = source;
    stale["schema_digest"] =
        json!("fbfb18279b0c7ebb350ef0ce0d56547fa11e83985b13380cb2b0f1dba4cb56e9");
    assert!(super::observation(&stale, "state_response", &super::config()).is_err());
    let mut settled = read_json("golden/dispatch-action-settled.json")?;
    settled["transition"] = Value::Null;
    assert!(!validator.is_valid(&settled));
    let action = EpisodeLegalAction::new("combat.end-turn", ActionKind::EndTurn)?;
    assert!(
        super::receipt(
            &settled,
            &settled.to_string(),
            "dispatch_action_response",
            &super::config(),
            "op-1",
            0,
            action,
        )
        .is_err()
    );
    Ok(())
}
