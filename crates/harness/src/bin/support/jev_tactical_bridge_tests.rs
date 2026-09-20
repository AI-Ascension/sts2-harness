// SPDX-License-Identifier: MIT

use super::tactical_fixture as fixture;
use super::*;

fn input() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "model_execution_id": "model-execution-7", "objective": "survive the turn",
        "hard_constraints": ["use only admitted facts"],
        "legal_action_ids": ["action-00", "action-01"],
        "observation": {
            "state_id": "combat-1", "generation": 3,
            "player": {"hp": 30, "max_hp": 80, "energy": 3, "gold": 0, "hand": []},
            "state": {"state": "combat", "turn_index": 2},
        },
    }))
    .expect("synthetic input")
}

#[test]
fn tactical_mode_uses_one_exchange_and_records_the_selected_evidence() {
    let mut calls = 0;
    let result = record_profile(
        &input(),
        "jev-latest",
        0.2,
        &mut |bytes| {
            calls += 1;
            let body: Value = serde_json::from_slice(bytes)?;
            assert_eq!(body["questions"].as_object().expect("questions").len(), 15);
            Ok(serde_json::to_vec(&fixture::reply(&body, "action-01"))?)
        },
        true,
    )
    .expect("tactical record");
    assert_eq!(calls, 1);
    assert_eq!(result["decision"]["action_id"], "action-01");
    assert_eq!(result["tactical"]["applied"], true);
    assert_eq!(
        result["tactical"]["assessment"]["response_model"],
        "jev-1.13.0"
    );
    assert_eq!(
        result["tactical"]["question_set_digest"]
            .as_str()
            .expect("digest")
            .len(),
        64
    );
}

#[test]
fn explicit_disabled_profile_matches_the_legacy_record() {
    let mut baseline_request = Vec::new();
    let baseline = record(&input(), "jev-latest", 0.2, &mut |bytes| {
        baseline_request = bytes.to_vec();
        let body: Value = serde_json::from_slice(bytes)?;
        Ok(serde_json::to_vec(&fixture::reply(&body, "action-00"))?)
    })
    .expect("legacy record");
    let disabled = record_profile(
        &input(),
        "jev-latest",
        0.2,
        &mut |bytes| {
            assert_eq!(bytes, baseline_request);
            let body: Value = serde_json::from_slice(bytes)?;
            assert_eq!(body["questions"].as_object().expect("questions").len(), 1);
            Ok(serde_json::to_vec(&fixture::reply(&body, "action-00"))?)
        },
        false,
    )
    .expect("disabled profile");
    assert_eq!(baseline, disabled);
    assert!(disabled.get("tactical").is_none());
}

#[test]
fn malformed_tactical_reply_is_not_retried() {
    let mut calls = 0;
    let result = record_profile(
        &input(),
        "jev-latest",
        0.2,
        &mut |_| {
            calls += 1;
            Ok(b"{\"answers\":{}}".to_vec())
        },
        true,
    );
    assert!(result.is_err());
    assert_eq!(calls, 1);
}

#[test]
fn tactical_duplicate_catalog_is_rejected_before_egress() {
    let mut request: Value = serde_json::from_slice(&input()).expect("request");
    request["legal_action_ids"] = json!(["same", "same"]);
    let mut calls = 0;
    let result = record_profile(
        &serde_json::to_vec(&request).expect("request bytes"),
        "jev-latest",
        0.2,
        &mut |_| {
            calls += 1;
            Ok(Vec::new())
        },
        true,
    );
    assert!(result.is_err());
    assert_eq!(calls, 0);
}
