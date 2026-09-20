// SPDX-License-Identifier: MIT

use super::*;
use super::super::mcp_process::McpProcessErrorKind;

#[test]
fn only_known_transport_failures_are_retryable_for_catalog_reads() {
    for kind in [
        McpProcessErrorKind::Deadline,
        McpProcessErrorKind::RequestWrite,
        McpProcessErrorKind::RequestFlush,
        McpProcessErrorKind::ResponseRead,
        McpProcessErrorKind::ResponseEof,
        McpProcessErrorKind::ProcessClosed,
        McpProcessErrorKind::StdinClosed,
        McpProcessErrorKind::StdoutClosed,
        McpProcessErrorKind::SupervisorClosed,
        McpProcessErrorKind::SupervisorUnavailable,
        McpProcessErrorKind::SupervisorFailed,
    ] {
        assert!(RpcFailure::from_mcp(McpProcessError::new(kind, "test")).is_transient());
    }
    assert!(!RpcFailure::from_mcp(McpProcessError::new(
        McpProcessErrorKind::Protocol,
        "test",
    ))
    .is_transient());
}

#[test]
fn gateway_server_errors_retry_only_timeout_or_unavailable() {
    for code in [-32003, -32008] {
        assert!(is_transient_gateway_rpc_error(
            &json!({"error":{"code":code}})
        ));
    }
    assert!(!is_transient_gateway_rpc_error(
        &json!({"error":{"code":-32002}})
    ));
    for text in [
        "gateway error -32003: gateway is unavailable",
        "gateway error -32008: gateway request timed out",
    ] {
        assert!(is_transient_gateway_tool_error(&json!({
            "result":{"content":[{"text":text}]}
        })));
    }
    assert!(!is_transient_gateway_tool_error(&json!({
        "result":{"content":[{"text":"gateway error -32002: gateway returned an invalid response"}]}
    })));
    assert!(!is_transient_gateway_tool_error(&json!({
        "result":{"content":[{"text":"gateway error -32008: gateway request timed out"}, {"text":"extra"}]}
    })));
}

#[test]
fn catalog_recovery_requires_exact_shape_and_correlation() {
    for code in [
        "stale_generation",
        "host_not_configured",
        "host_observation_unavailable",
        // A refused launch contract answers with the mod's refusal code rather than the
        // never-declared code, and it must be admitted the same way.
        "launch_contract_refused",
        "launch_contract_refused_isolated_user_dir_mismatch",
        "launch_contract_refused_campaign_required",
    ] {
        let mut body =
            json!({"correlation_id":"42", "error_code":code, "recovery":"reobserve"});
        let response = |body: &Value| json!({"result":{"isError":true,"content":[{"text":body.to_string()}]}});
        assert!(has_catalog_reobserve(&response(&body), 42));
        assert!(!has_catalog_reobserve(&response(&body), 43));
        body["private"] = json!("extra");
        assert!(!has_catalog_reobserve(&response(&body), 42));
    }
    for code in ["unauthorized", "timeout", "unknown"] {
        assert!(!catalog_reobserve(
            &json!({"correlation_id":"42","error_code":code,"recovery":"reobserve"})
        ));
    }
}

/// The admitted refusal shape is the producer's own rule, so a code the mod cannot compose must
/// stay refused here rather than be admitted as a neighbouring string.
#[test]
fn a_refused_launch_contract_code_is_admitted_only_in_the_shape_the_mod_composes() {
    let admitted = |code: &str| {
        catalog_reobserve(&json!({
            "correlation_id":"42", "error_code":code, "recovery":"reobserve"}))
    };
    let longest = format!("launch_contract_refused_{}", "a".repeat(64));
    for code in [
        "launch_contract_refused",
        "launch_contract_refused_a",
        "launch_contract_refused_isolated_user_dir_mismatch",
        "launch_contract_refused_campaign_required",
        "launch_contract_refused_UPPER_lower-123_456",
        // `_` is a legal token character, so a token may begin with one.
        "launch_contract_refused__leading",
        longest.as_str(),
    ] {
        assert!(admitted(code), "the refusal code {code} must be admitted");
    }
    for code in [
        "launch_contract_refused_",
        "launch_contract_refused_..",
        "launch_contract_refused_a b",
        "launch_contract_refused_a.b",
        "launch_contract_refused_a/b",
        "launch_contract_refused_ünicode",
        "launch_contract_refusedx",
        "launch_contractrefused",
        "launch_contract",
        "host_not_configured_refused",
    ] {
        assert!(!admitted(code), "{code} must stay refused");
    }
    // One byte over the reason bound is the first token the producer degrades to the bare prefix,
    // so the neighbouring admitted code is the 64-byte token and not this.
    assert!(!admitted(&format!(
        "launch_contract_refused_{}",
        "a".repeat(65)
    )));
    // The refusal is admitted only in the refusal's own envelope shape.
    let refusal = json!({"correlation_id":"42",
        "error_code":"launch_contract_refused_isolated_user_dir_mismatch",
        "recovery":"reobserve"});
    let response =
        json!({"result":{"isError":true,"content":[{"text":refusal.to_string()}]}});
    assert!(has_catalog_reobserve(&response, 42));
    assert!(!has_catalog_reobserve(&response, 43));
    let mut extra = refusal.clone();
    extra["private"] = json!("extra");
    assert!(!catalog_reobserve(&extra));
    let mut retry = refusal.clone();
    retry["recovery"] = json!("retry");
    assert!(!catalog_reobserve(&retry));
}
#[test]
fn gameplay_unknown_remains_available_for_receipt_validation() {
    let envelope = json!({"protocol_version":"runtime-v3-gameplay", "status":"unknown",
        "error_code":"settlement_unproven"});
    let mut response = json!({"result":{"isError":true,
        "content":[{"text":envelope.to_string()}]}});
    assert!(has_gameplay_envelope(&response));
    response["result"]["content"][0]["text"] = json!("gateway error -32005: rejected");
    assert!(!has_gameplay_envelope(&response));
    response["result"]["content"][0]["text"] = json!("{}");
    assert!(!has_gameplay_envelope(&response));
}
#[test]
fn transition_wait_budget_includes_requested_semantic_wait() -> Result<(), String> {
    let mut params =
        json!({"name":"sts2.wait_for_transition","arguments":{"wait_for_millis":120_000}});
    assert_eq!(request_timeout("tools/call", &params)?.as_millis(), 125_000);
    assert_eq!(request_timeout("initialize", &params)?.as_millis(), 5_000);
    params["arguments"]["wait_for_millis"] = json!(120_001);
    assert!(request_timeout("tools/call", &params).is_err());
    params["arguments"]["wait_for_millis"] = Value::Null;
    assert!(request_timeout("tools/call", &params).is_err());
    Ok(())
}

#[test]
fn bootstrap_error_envelope_is_preserved_but_other_bootstrap_tool_errors_are_not() {
    let tool_error = |text: &str| {
        serde_json::json!({"jsonrpc":"2.0","id":4,"result":{"isError":true,
            "content":[{"type":"text","text":text}]}})
    };
    let error_response = serde_json::json!({
        "protocol_version":"game-information-live-observation-bootstrap-v1",
        "kind":"error_response","error":{"code":"not_observable"}
    })
    .to_string();
    assert!(has_bootstrap_error_envelope(&tool_error(&error_response)));
    let success_shape = error_response.replace("error_response", "bootstrap_response");
    assert!(!has_bootstrap_error_envelope(&tool_error(&success_shape)));
    let foreign_protocol = error_response.replace("live-observation-bootstrap-v1", "query-v1");
    assert!(!has_bootstrap_error_envelope(&tool_error(&foreign_protocol)));
    assert!(!has_bootstrap_error_envelope(&tool_error(
        "gateway error -32005: gateway rejected the request"
    )));
}
