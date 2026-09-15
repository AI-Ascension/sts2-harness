// SPDX-License-Identifier: MIT

// Hand-authored MIT synthetic additions to the unchanged MIT protocol golden.
// No game files, real profiles, provider payloads or private identifiers.
use serde_json::{Value, json};

pub const GOLDEN: &str =
    include_str!("../../../../protocol-artifact/seeded-run-v1/golden/start-settled.json");

pub fn receipt() -> Value {
    json!({
        "operation_id": "op-seed-1", "requested_seed": "ironclad-42",
        "plan_digest": "a".repeat(64), "entry_ordinal": 0,
        "settled": serde_json::from_str::<Value>(GOLDEN).unwrap(),
        "start": null, "reconcile": []
    })
}

pub fn occurrence() -> Value {
    json!({
        "process_id": "process-1", "run_id": "run-1", "episode_id": "episode-1",
        "instance_id": "instance-1", "gateway_session_id": "session-1",
        "mcp_session_id": "mcp-session-1", "lease_id": "lease-1", "lease_epoch": 1,
        "operation_id": "op-seed-1", "request_generation": 0,
        "plan_digest": "a".repeat(64), "entry_ordinal": 0,
        "run_mode": "seeded_training", "created_at_unix_ms": 1
    })
}

pub fn document() -> Value {
    let context = receipt()["settled"]["selected_context"].clone();
    let component = json!({"revision": "1".repeat(40), "package_digest": "2".repeat(64)});
    let mut mod_component = component.clone();
    mod_component["package_digest"] = context["compatibility"]["mod"]["digest"].clone();
    json!({
        "version": "ascension.benchmark-manifest.v1",
        "gameplay": {
            "requested_seed": "ironclad-42", "effective_seed": "ironclad-42",
            "seed_contract": "synthetic-seed-contract-v1", "selected_context": context,
            "game_version": "synthetic-game-v1",
            "assembly_hashes": {"synthetic-assembly": "3".repeat(64)},
            "components": {"harness": component, "mcp": component, "gateway": component,
                "game_mod": mod_component},
            "protocol_version": "seeded-run-v1", "protocol_digest": "4".repeat(64),
            "coverage_version": "synthetic-coverage-v1", "coverage_digest": "5".repeat(64),
            "platform": {"os": "synthetic-os", "architecture": "x86_64",
                "runtime_version": "synthetic-runtime-v1", "compatibility_contract": "exact-equality-v1"},
            "profile_artifact": {"reference": "profile-artifact-1",
                "baseline_digest": context["profile_baseline"]["digest"]},
            "unlock_progress_digest": "6".repeat(64), "gameplay_settings_digest": "7".repeat(64)
        },
        "experiment": {
            "provider": "synthetic-provider", "model": "synthetic-model",
            "provider_revision": {"status": "unavailable"},
            "model_revision": {"status": "known", "value": "synthetic-model-v1"},
            "prompt_digest": "8".repeat(64), "workflow_digest": "9".repeat(64),
            "context_digest": "a".repeat(64), "tool_policy_digest": "b".repeat(64),
            "inference_parameters": {"temperature": "0.5", "top_p": "1"},
            "inference_seed": {"status": "unavailable"},
            "budgets": {"max_decisions": 100, "max_tokens": 10000, "max_duration_ms": 60000},
            "evaluator_revision": "synthetic-evaluator-v1"
        }
    })
}

// Build modified protocol fixture bytes from the original *ordered* golden input.
// The expected context digest hashes literal fixture bytes, independently of the
// production typed context serializer.
pub fn context_variant(from: &str, to: &str) -> Value {
    let prefix = GOLDEN
        .split("\"selected_context\":")
        .nth(1)
        .unwrap()
        .split(",\"context_digest\":")
        .next()
        .unwrap()
        .replace(from, to);
    let digest = sts2_harness::sha256_hex(format!("{prefix}}}"));
    serde_json::from_str(&format!("{prefix},\"context_digest\":\"{digest}\"}}")).unwrap()
}

pub fn bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}
