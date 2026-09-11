// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;
use crate::runtime_support::seed_transport::SeedTransportConfig;

fn observation(stage: &str, generation: u64, id: &str) -> Value {
    json!({"state_id":format!("state-{generation}"),"generation":generation,
        "visible_seed":"ironclad-42","player":{"hp":80,"max_hp":80,"energy":3,"gold":99,
        "deck":[],"hand":[],"discard":[],"exhaust":[]},
        "state":{"state":stage,"characters":["ironclad"]},
        "legal_actions":[{"action_id":id,"action":{"kind":"start_run","character_id":"ironclad"}}]})
}

fn response(value: Value, id: u64) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":{"content":[{
        "type":"text","text":value.to_string()
    }],"isError":false}})
}

fn rows() -> Vec<Value> {
    let start: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/seeded-run-v1/golden/start-accepted.json"
    ))
    .expect("synthetic accepted receipt");
    let mut settled: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/seeded-run-v1/golden/start-settled.json"
    ))
    .expect("synthetic settled receipt");
    settled["kind"] = json!("reconcile_response");
    settled["observation"]["phase_before"] = json!("campaign_setup");
    let mut terminal = observation("defeat", 3, "unused");
    terminal["state"] = json!({"state":"defeat","reason":null});
    terminal["player"]["hp"] = json!(0);
    terminal["legal_actions"] = json!([]);
    vec![
        json!({"event":"seeded_run_receipt","receipt":{"operation_id":"op-seed-1",
            "requested_seed":"ironclad-42","plan_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "entry_ordinal":0,"start":response(start, 1),"reconcile":[response(settled.clone(), 2)],
            "duplicate_start":response({ let mut copy = settled.clone(); copy["kind"] = json!("start_response"); copy }, 3),"settled":settled}}),
        json!({"event":"model_decision","action_id":"start-1","observation":observation("map", 1, "start-1")}),
        json!({"event":"action_receipt","action_id":"start-1","operation_id":"op-1","status":"Unknown"}),
        json!({"event":"operation_wait_completed","operation_id":"op-1","effect":"test_terminal","observation":terminal}),
        json!({"event":"episode_complete","observation":terminal}),
        json!({"protocol":"runtime-v3-gameplay","status":"complete"}),
    ]
}

fn parse(values: &[Value]) -> Result<ReplayTrace, String> {
    let text = values
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    ReplayTrace::parse(text.as_bytes())
}

#[test]
fn seeded_receipt_admits_only_matching_current_config() {
    let trace = parse(&rows()).expect("proven seeded receipt replay");
    let seed = SeedTransportConfig::fixture_for_tests("ironclad-42", "op-seed-1");
    assert!(trace.admit_seeded_receipt(Some(&seed)).is_ok());
    assert!(trace.admit_seeded_receipt(None).is_err());
    for dimension in [
        "seed",
        "operation",
        "plan",
        "ordinal",
        "mode",
        "context",
        "profile",
        "game_compatibility",
        "mod_compatibility",
        "context_digest",
    ] {
        let mut changed = if matches!(
            dimension,
            "context" | "profile" | "game_compatibility" | "mod_compatibility" | "context_digest"
        ) {
            SeedTransportConfig::fixture_with_context_mismatch(dimension)
        } else {
            SeedTransportConfig::fixture_for_tests("ironclad-42", "op-seed-1")
        };
        match dimension {
            "seed" => changed.requested_seed = "other-seed".into(),
            "operation" => changed.operation_id = "op-other".into(),
            "plan" => changed.plan_digest = "b".repeat(64),
            "ordinal" => changed.entry_ordinal = 1,
            "mode" => changed.run_mode = "diagnostic".into(),
            "context" | "profile" | "game_compatibility" | "mod_compatibility"
            | "context_digest" => {}
            _ => unreachable!(),
        }
        assert!(trace.admit_seeded_receipt(Some(&changed)).is_err());
    }
}

#[test]
fn seeded_receipt_rejects_malformed_mcp_wrappers() {
    for change in [
        "wrapper_extra_field",
        "result_error",
        "result_extra_field",
        "content_extra_field",
        "non_numeric_id",
        "non_text_content",
    ] {
        let mut values = rows();
        match change {
            "wrapper_extra_field" => values[0]["receipt"]["start"]["error"] = json!("unexpected"),
            "result_error" => values[0]["receipt"]["start"]["result"]["isError"] = json!(true),
            "result_extra_field" => values[0]["receipt"]["start"]["result"]["extra"] = json!(true),
            "content_extra_field" => {
                values[0]["receipt"]["start"]["result"]["content"][0]["extra"] = json!(true)
            }
            "non_numeric_id" => values[0]["receipt"]["start"]["id"] = json!("one"),
            "non_text_content" => {
                values[0]["receipt"]["start"]["result"]["content"][0]["type"] = json!("image")
            }
            _ => unreachable!(),
        }
        assert!(parse(&values).is_err(), "{change} must fail closed");
    }
}

#[test]
fn seeded_receipt_limits_error_wrappers_to_unknown_recovery() {
    let mut values = rows();
    let mut unknown: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/seeded-run-v1/golden/start-accepted.json"
    ))
    .expect("synthetic accepted receipt");
    unknown["kind"] = json!("reconcile_response");
    unknown["status"] = json!("unknown");
    unknown["error_code"] = json!("transport_timeout");
    let mut wrapper = response(unknown, 4);
    wrapper["result"]["isError"] = json!(true);
    values[0]["receipt"]["reconcile"]
        .as_array_mut()
        .expect("synthetic reconciliation array")
        .insert(0, wrapper);
    assert!(parse(&values).is_ok());

    values[0]["receipt"]["reconcile"][1]["result"]["isError"] = json!(true);
    assert!(parse(&values).is_err());
}

#[test]
fn seeded_receipt_rejects_tampering_and_arbitrary_map_sources() {
    for change in [
        "partial",
        "operation",
        "seed",
        "context",
        "fence",
        "resumed",
        "arbitrary_map",
    ] {
        let mut values = rows();
        match change {
            "partial" => values[0]["receipt"]["reconcile"] = json!([]),
            "operation" => values[0]["receipt"]["settled"]["operation_id"] = json!("other"),
            "seed" => values[1]["observation"]["visible_seed"] = json!("other-seed"),
            "context" => {
                values[0]["receipt"]["settled"]["selected_context"]["profile_baseline"]["identity"] =
                    json!("other-profile")
            }
            "fence" => values[0]["receipt"]["settled"]["lease_epoch"] = json!(2),
            "resumed" => {
                values[0]["receipt"]["settled"]["observation"]["phase_before"] = json!("combat")
            }
            "arbitrary_map" => values[1]["observation"]["generation"] = json!(2),
            _ => unreachable!(),
        }
        assert!(parse(&values).is_err(), "{change} must fail closed");
    }
}

#[test]
fn seeded_receipt_rejects_a_prior_skipped_rejected_attempt() {
    let mut values = rows();
    let rejected_decision = values[1].clone();
    let rejected = json!({
        "event":"action_receipt",
        "action_id":"start-1",
        "operation_id":"not-dispatched",
        "status":"Rejected",
        "effect":null,
        "observation":rejected_decision["observation"]
    });
    values.insert(0, rejected_decision);
    values.insert(1, rejected);

    assert!(
        parse(&values).is_err(),
        "a seeded receipt must be the first replay record, not follow a skipped rejection"
    );
}

#[test]
fn private_seeded_receipt_source_parser_validation() {
    let Ok(path) = std::env::var("STS2_PRIVATE_SEEDED_REPLAY_SOURCE") else {
        return;
    };
    let bytes = std::fs::read(path).expect("private replay source must be readable");
    let trace = ReplayTrace::parse(&bytes).expect("private seeded receipt source must parse");
    let seed = SeedTransportConfig::from_environment()
        .expect("private replay seed configuration must parse")
        .expect("private replay must provide a seed configuration");
    trace
        .admit_seeded_receipt(Some(&seed))
        .expect("private source receipt must match replay seed configuration");
}
