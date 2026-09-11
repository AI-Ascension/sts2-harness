// SPDX-License-Identifier: MIT

use super::recorded_run_support::token;
use serde_json::{Value, json};

// Source-owned envelope generation is the request fence, not the resulting generation.
// These are structural/internal receipt checks; no offline adapter can authenticate the host.
pub(super) fn valid(receipt: &Value) -> bool {
    let v = &receipt["settled"];
    if v["protocol_version"] != "seeded-run-v1"
        || v["schema_digest"] != "5c659f344be78f84e8d783986925d462714f933cac95d18943358992f7d3e2b8"
    {
        return false;
    }
    if v["provenance"]
        != json!({"artifact":"sts2-protocol/seeded-run-v1",
        "source":"schemas/seeded-run-v1.schema.json","generator":"hand-authored"})
        || !matches!(
            v["kind"].as_str(),
            Some("start_response" | "reconcile_response")
        )
        || v["status"] != "settled"
        || !v["error_code"].is_null()
    {
        return false;
    }
    for field in [
        "instance_id",
        "session_id",
        "lease_id",
        "operation_id",
        "correlation_id",
    ] {
        if !v[field].as_str().is_some_and(|s| token(s, 128)) {
            return false;
        }
    }
    for field in ["requested_seed", "operation_id"] {
        if receipt.get(field).is_some_and(|value| value != &v[field]) {
            return false;
        }
    }
    let Some(request) = v["generation"].as_u64() else {
        return false;
    };
    let Some(observed) = v["observation"]["generation"].as_u64() else {
        return false;
    };
    let o = &v["observation"];
    let w = &v["effect_witness"];
    let Some(seed) = v["canonical_seed"].as_str() else {
        return false;
    };
    let Some(context) = v["context_digest"].as_str() else {
        return false;
    };
    !seed.is_empty()
        && seed.len() <= 64
        && observed > request
        && observed <= 9_007_199_254_740_991
        && w["generation"] == o["generation"]
        && w["canonical_seed"] == seed
        && o["canonical_seed"] == seed
        && w["kind"] == "run_started"
        && o["run_started"] == true
        && o["host_ready"] == true
        && o["selected_context_digest"] == context
        && v["selected_context"]["context_digest"] == context
        && o["phase_before"] != o["phase_after"]
        && ["phase_before", "phase_after", "compatibility_identity"]
            .iter()
            .all(|field| o[field].as_str().is_some_and(|s| token(s, 128)))
}
