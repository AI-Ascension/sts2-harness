// SPDX-License-Identifier: MIT

use super::*;

pub(crate) fn assert_success(
    result: &ScenarioResult,
) -> Result<String, Box<dyn std::error::Error>> {
    if result.runtime.status.code() != Some(0) {
        return Err(format!(
            "runtime failed: {}",
            String::from_utf8_lossy(&result.runtime.stderr)
        )
        .into());
    }
    if !result.ledger.errors.is_empty() {
        return Err(format!("fixture failed: {:?}", result.ledger.errors).into());
    }
    let action = result
        .ledger
        .requests
        .iter()
        .find(|request| request.path == "/api/v4/runtime/expert-action")
        .ok_or("expert action missing")?;
    let operation = action.body["operation_id"]
        .as_str()
        .ok_or("operation missing")?
        .to_owned();
    let expected = [
        "/api/v3/runtime/state",
        "/api/v4/runtime/expert-state",
        "/api/v3/runtime/legal-actions",
        "/api/v4/runtime/expert-state",
        "/api/v4/runtime/expert-action",
    ];
    let actual = paths(&result.ledger);
    if actual.len() != 6
        || actual[..5] != expected
        || actual[5] != format!("/api/v4/runtime/expert-actions/{operation}")
    {
        return Err(format!("unexpected path ledger: {actual:?}").into());
    }
    let methods: Vec<&str> = result
        .ledger
        .requests
        .iter()
        .map(|request| request.method.as_str())
        .collect();
    if methods != ["GET", "GET", "GET", "GET", "POST", "GET"] {
        return Err(format!("unexpected downstream methods: {methods:?}").into());
    }
    let statuses: Vec<u16> = result
        .ledger
        .responses
        .iter()
        .map(|response| response.status)
        .collect();
    if statuses != [200, 200, 200, 200, 503, 200] {
        return Err(format!("unexpected downstream response statuses: {statuses:?}").into());
    }
    if action.body["state_id"] != "live:7"
        || action.body["generation"] != 7
        || action.body["action"]["action_id"] != ACTION_ID
        || action.body["action"]["action"]["kind"] != "use_potion"
        || action.body["status"] != Value::Null
    {
        return Err("action fence mismatch".into());
    }
    let reconcile = &result.ledger.requests[5];
    if reconcile.body != Value::Null
        || reconcile.headers.get("x-sts2-lease-id").map(String::as_str) != Some(LEASE_ID)
        || reconcile
            .headers
            .get("x-sts2-lease-epoch")
            .map(String::as_str)
            != Some("1")
    {
        return Err("reconcile lease mismatch".into());
    }
    let unknown = &result.ledger.responses[4].body;
    if unknown["status"] != "unknown"
        || unknown["operation_id"] != operation
        || unknown["state_id"] != "live:7"
        || unknown["generation"] != 7
    {
        return Err("unknown response identity mismatch".into());
    }
    let settled = &result.ledger.responses[5].body;
    if settled["status"] != "settled"
        || settled["operation_id"] != operation
        || settled["state_id"] != "live:8"
        || settled["generation"] != 8
        || settled["observation"]["state_id"] != "live:8"
        || settled["observation"]["generation"] != 8
    {
        return Err("settled response identity mismatch".into());
    }
    let report: Value = serde_json::from_slice(&result.runtime.stdout)
        .map_err(|error| format!("runtime report is not JSON: {error}"))?;
    if report["protocol"] != "runtime-v4-expert"
        || report["status"] != "complete"
        || report["terminal_stage"] != "victory"
        || report["final_generation"] != 8
        || report["transitions"] != 1
    {
        return Err(format!("runtime completion report mismatch: {report}").into());
    }
    Ok(operation)
}

pub(crate) fn assert_foreign_state_rejected(
    result: &ScenarioResult,
) -> Result<(), Box<dyn std::error::Error>> {
    if result.runtime.status.code() != Some(2) {
        return Err(format!("foreign state exit: {:?}", result.runtime.status.code()).into());
    }
    let methods: Vec<&str> = result
        .ledger
        .requests
        .iter()
        .map(|request| request.method.as_str())
        .collect();
    if !result.ledger.errors.is_empty()
        || paths(&result.ledger) != ["/api/v3/runtime/state", "/api/v4/runtime/expert-state"]
        || methods != ["GET", "GET"]
        || result.ledger.responses.len() != 2
        || result.ledger.responses[0].status != 200
        || result.ledger.responses[1].status != 200
        || result.ledger.responses[1].body["state_id"] != "foreign-state"
    {
        return Err(format!(
            "foreign state was not rejected at composition: exit={:?}, errors={:?}, paths={:?}, methods={methods:?}, responses={:?}, stderr={}",
            result.runtime.status.code(),
            result.ledger.errors,
            paths(&result.ledger),
            result.ledger.responses,
            String::from_utf8_lossy(&result.runtime.stderr),
        )
        .into());
    }
    Ok(())
}
