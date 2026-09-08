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
