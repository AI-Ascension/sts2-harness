// SPDX-License-Identifier: MIT

#![allow(clippy::panic)]

use serde_json::json;

use super::super::super::config::RuntimeConfig;
use super::{validate_seeded_response, validate_seeded_settlement};

fn config() -> RuntimeConfig {
    RuntimeConfig {
        seed_transport: Some(
            super::super::super::seed_transport::SeedTransportConfig::fixture_for_tests(
                "ironclad-42",
                "op-seed-1",
            ),
        ),
        gateway_address: String::from("127.0.0.1:15525"),
        gateway_token: String::from("synthetic-token"),
        mcp_binary: String::from("unused-test-binary"),
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
        trace_id: String::from("trace-1"),
        artifact_id: String::from("artifact-1"),
        wait_for_combat_seconds: 0,
        settlement_timeout_seconds: 30,
    }
}

fn settled() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/seeded-run-v1/golden/start-settled.json"
    ))
    .unwrap_or_else(|error| panic!("settled seeded-run golden must parse: {error}"))
}

#[test]
fn settled_response_accepts_native_canonical_seed_but_requires_exact_request_echo() {
    let config = config();
    let seed = config
        .seed_transport
        .as_ref()
        .unwrap_or_else(|| panic!("test seed config must be present"));
    let mut value = settled();
    value["canonical_seed"] = json!("Ironclad-42");
    value["observation"]["canonical_seed"] = json!("Ironclad-42");
    value["effect_witness"]["canonical_seed"] = json!("Ironclad-42");
    assert!(validate_seeded_settlement(&value, &config, seed, 0, "start_response").is_ok());

    value["requested_seed"] = json!("Ironclad-42");
    assert!(validate_seeded_settlement(&value, &config, seed, 0, "start_response").is_err());
}

#[test]
fn settled_response_binds_protocol_identity_context_generation_and_witness() {
    let config = config();
    let seed = config
        .seed_transport
        .as_ref()
        .unwrap_or_else(|| panic!("test seed config must be present"));
    let baseline = settled();

    for (field, replacement) in [
        ("instance_id", json!("foreign-instance")),
        ("session_id", json!("foreign-session")),
        ("lease_id", json!("foreign-lease")),
        ("operation_id", json!("foreign-operation")),
        ("context_digest", json!("b".repeat(64))),
        ("schema_digest", json!("b".repeat(64))),
        ("generation", json!(1)),
    ] {
        let mut value = baseline.clone();
        value[field] = replacement;
        assert!(
            validate_seeded_response(&value, &config, seed, 0, "start_response").is_err(),
            "{field} mismatch must fail closed"
        );
    }

    let mut value = baseline.clone();
    value["selected_context"]["selection_policy"] = json!("other");
    assert!(validate_seeded_response(&value, &config, seed, 0, "start_response").is_err());

    let mut value = baseline.clone();
    value["observation"]["generation"] = json!(0);
    assert!(validate_seeded_settlement(&value, &config, seed, 0, "start_response").is_err());

    let mut value = baseline;
    value["effect_witness"]["generation"] = json!(2);
    assert!(validate_seeded_settlement(&value, &config, seed, 0, "start_response").is_err());
}

#[test]
fn accepted_response_uses_the_same_strict_protocol_envelope() {
    let config = config();
    let seed = config
        .seed_transport
        .as_ref()
        .unwrap_or_else(|| panic!("test seed config must be present"));
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/seeded-run-v1/golden/start-accepted.json"
    ))
    .unwrap_or_else(|error| panic!("accepted seeded-run golden must parse: {error}"));
    assert!(validate_seeded_response(&value, &config, seed, 0, "start_response").is_ok());
}
