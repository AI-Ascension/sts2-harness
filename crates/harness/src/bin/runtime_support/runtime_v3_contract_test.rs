// SPDX-License-Identifier: MIT

use std::{fs, path::PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
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
        format!("{:x}", Sha256::digest(&sums)),
        "ec17dc526545c356462773f9e634ea7b25546c877c601cc1640eae3d7341cb81"
    );
    let sums = String::from_utf8(sums)?;
    assert_eq!(sums.lines().count(), 8);
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
            format!("{:x}", Sha256::digest(fs::read(artifact().join(local))?)),
            digest
        );
    }
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(fs::read(artifact().join("schema.json"))?)
        ),
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
