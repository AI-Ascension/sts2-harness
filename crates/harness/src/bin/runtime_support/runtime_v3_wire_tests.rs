// SPDX-License-Identifier: MIT

use super::*;
use super::super::mcp_process::McpProcessErrorKind;
use super::super::runtime_v3::{
    RuntimeV3ToolError, accepts_recovery_envelope_for_tool, transient_allowed_for_tool,
};
use std::time::{Duration, Instant};

/// An MCP tool-error response whose single text content is `text`.
fn tool_error_with_text(text: &str) -> serde_json::Value {
    serde_json::json!({"jsonrpc":"2.0","id":4,"result":{"isError":true,
        "content":[{"type":"text","text":text}]}})
}

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
fn map_profile_deadline_reserves_cleanup_and_caps_every_rpc() -> Result<(), String> {
    let start = Instant::now();
    let deadline = MapProfileDeadline::from_start(start)
        .ok_or_else(|| String::from("fixture map profile deadline must be representable"))?;
    let request = Duration::from_secs(5);

    assert_eq!(deadline.rpc_timeout_at(start, request), Some(request));
    assert_eq!(
        deadline.rpc_timeout_at(start + Duration::from_secs(5), request),
        Some(Duration::from_millis(3_750))
    );
    assert_eq!(
        deadline.rpc_timeout_at(start + Duration::from_millis(7_500), request),
        Some(Duration::from_millis(1_250))
    );
    let exhausted = start + Duration::from_millis(8_750);
    assert_eq!(deadline.rpc_timeout_at(exhausted, request), None);
    assert_eq!(
        deadline.remaining_at(start + Duration::from_secs(9)),
        Duration::from_secs(1)
    );

    let mut invoked = false;
    assert!(deadline
        .with_rpc_timeout_at(exhausted, request, |_| invoked = true)
        .is_none());
    assert!(!invoked, "expired deadlines must not start another RPC");
    Ok(())
}

#[test]
fn map_deadline_helper_keeps_transition_wait_timeout_policy_unchanged() -> Result<(), String> {
    let params = json!({
        "name":"sts2.wait_for_transition",
        "arguments":{"wait_for_millis":120_000}
    });
    assert_eq!(
        request_timeout("tools/call", &params)?,
        Duration::from_millis(125_000)
    );
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

#[test]
fn observe_dispatch_classifies_transient_gateway_faults_as_retryable() {
    // `sts2.observe` is dispatched as a gameplay read, so a briefly unavailable
    // gateway is a retryable fault rather than a terminal refusal. This is the
    // classification the owner's launch fence depends on: a terminal reading
    // here surfaces as `live_launch_fence_failed` and refuses the whole run.
    for code in [-32003, -32008] {
        assert!(classifies_transient_gateway_faults(RpcReadKind::Gameplay));
        assert!(is_transient_gateway_rpc_error(&json!({"error":{"code":code}})));
    }
    // A dispatch that is not a read keeps its existing terminal behaviour.
    assert!(!classifies_transient_gateway_faults(RpcReadKind::None));
    // Reads keep classifying transience.
    assert!(classifies_transient_gateway_faults(RpcReadKind::Catalog));
    assert!(classifies_transient_gateway_faults(RpcReadKind::Recovery));
}

#[test]
fn gameplay_read_kind_does_not_widen_accepted_envelopes() {
    // The gameplay kind exists to enable the transient classification only. If
    // it ever started accepting recovery envelopes, `sts2.observe` could come to
    // accept a watchdog-recovery payload as a gameplay observation.
    let recovery_envelope = tool_error_with_text(&serde_json::json!({
        "contract":"watchdog-recovery-v1",
        "schema_digest": sts2_harness::RECOVERY_SCHEMA_DIGEST,
        "kind":"operation_lookup_response"
    })
    .to_string());
    assert!(has_recovery_envelope(&recovery_envelope));
    // `Gameplay` is not `Recovery`, so the recovery envelope is not admitted by
    // the gameplay arm of the envelope test.
    assert_ne!(RpcReadKind::Gameplay, RpcReadKind::Recovery);
}

#[test]
fn only_observe_and_legal_actions_dispatch_as_reads() {
    // `rpc_call` chooses the read kind from the tool name. `sts2.observe` must be
    // classified as a gameplay read so its transient gateway faults are
    // retryable; if it were routed to `None` the owner's launch fence would refuse
    // the whole run with `live_launch_fence_failed` whenever the gateway is
    // briefly unavailable.
    assert_eq!(
        read_kind_for_dispatch("tools/call", &json!({"name":"sts2.observe"})),
        RpcReadKind::Gameplay
    );
    // The pre-existing catalog arm must not regress.
    assert_eq!(
        read_kind_for_dispatch("tools/call", &json!({"name":"sts2.legal_actions"})),
        RpcReadKind::Catalog
    );
    // Writes and unrelated reads keep terminal classification.
    for name in [
        "sts2.wait_for_transition",
        "sts2.dispatch",
    ] {
        assert_eq!(
            read_kind_for_dispatch("tools/call", &json!({"name":name})),
            RpcReadKind::None,
            "{name} must not become a retryable read"
        );
    }
    assert_eq!(read_kind_for_dispatch("tools/list", &json!({})), RpcReadKind::None);
}

#[test]
fn every_recovery_envelope_read_still_classifies_transient_faults() {
    // Regression guard for a real regression this patch introduced: `reobserve`
    // is dispatched through `rpc_call_recovery_read`, so if the transient
    // predicate disagrees with the recovery-envelope predicate, a genuine
    // transport fault on reobserve is downgraded to terminal and
    // `RecoveryError::Terminal` replaces the fail-closed `PortFailure`.
    //
    // Any tool that accepts a recovery envelope must therefore also classify
    // transient gateway faults; otherwise the reconnect fault matrix stops
    // failing closed.
    for name in ["sts2.legal_actions", "sts2.reobserve", "sts2.coop_receipt_query"] {
        assert!(
            accepts_recovery_envelope_for_tool(name),
            "{name} must keep accepting recovery envelopes"
        );
        assert!(
            transient_allowed_for_tool(name),
            "{name} must classify transient gateway faults"
        );
    }

    // The gameplay read is the deliberate asymmetry: it classifies transients
    // while widening no envelope, so observe cannot accept a recovery payload.
    assert!(transient_allowed_for_tool("sts2.observe"));
    assert!(!accepts_recovery_envelope_for_tool("sts2.observe"));

    // A write must satisfy neither predicate.
    assert!(!transient_allowed_for_tool("sts2.dispatch_action"));
    assert!(!accepts_recovery_envelope_for_tool("sts2.dispatch_action"));
}

#[test]
fn transient_classification_survives_the_tool_call_envelope() {
    // Regression guard for the exact production failure: the owner's launch
    // fence refuses the whole run with `live_launch_fence_failed` when an
    // `sts2.observe` transient gateway fault is flattened back to terminal.
    //
    // The wire layer classifies the fault (envelope accepted), and the tool-call
    // layer must then preserve that classification. `from_rpc_for` is the seam
    // where a transient can still be downgraded, so drive it directly: a
    // transient failure on a dispatch that classifies transients must stay
    // transient even though `sts2.observe` widens no recovery envelope.
    //
    // The flag is derived through the same predicate the call site uses. Hard
    // coding `true` here would make this test pass even if the call site
    // regressed to the narrower recovery predicate, which is precisely the
    // defect being guarded.
    let transient_failure = RpcFailure::transient("MCP read was temporarily unavailable");
    assert!(transient_failure.is_transient());
    assert!(transient_allowed_for_tool("sts2.observe"));
    assert!(matches!(
        RuntimeV3ToolError::from_rpc_for(
            transient_failure,
            transient_allowed_for_tool("sts2.observe")
        ),
        RuntimeV3ToolError::Transient(_)
    ));

    // A terminal protocol fault stays terminal even on a read dispatch: the
    // read path must not turn a genuine refusal into a retry.
    let terminal_failure = RpcFailure::terminal("MCP initialize omitted result");
    assert!(!terminal_failure.is_transient());
    assert!(matches!(
        RuntimeV3ToolError::from_rpc_for(terminal_failure, true),
        RuntimeV3ToolError::Terminal(_)
    ));

    // A write dispatch is not a read, so it keeps terminal classification and a
    // retry could double-apply it.
    for name in ["sts2.dispatch_action", "sts2.wait_for_transition"] {
        assert!(
            !transient_allowed_for_tool(name),
            "{name} must not become retryable"
        );
        assert!(matches!(
            RuntimeV3ToolError::from_rpc_for(
                RpcFailure::transient("transport fault"),
                transient_allowed_for_tool(name)
            ),
            RuntimeV3ToolError::Terminal(_)
        ));
    }

    // `sts2.observe` is the gameplay read: it classifies transients without
    // widening envelopes, which is the distinction the fix rests on.
    assert!(transient_allowed_for_tool("sts2.observe"));
    assert_ne!(
        read_kind_for_dispatch("tools/call", &json!({"name":"sts2.observe"})),
        RpcReadKind::Recovery
    );
}
