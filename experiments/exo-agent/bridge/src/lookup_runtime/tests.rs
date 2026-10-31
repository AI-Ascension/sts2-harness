// SPDX-License-Identifier: MIT
use super::*;
use serde_json::json;
#[test]
fn only_owned_names_closed_reads_and_bounds_are_forwarded() -> Result<(), serde_json::Error> {
    let mut request: ToolRequest = serde_json::from_value(json!({
        "function_name":"sts2_lookup_read","arguments":{"record_ordinal":1,"offset":2048}
    }))?;
    assert!(matches!(
        tool_payload(&request),
        Ok(Payload::ReadRetained { offset: 2048, .. })
    ));
    request.arguments.insert("scope".into(), json!("research"));
    assert!(tool_payload(&request).is_err());
    request.arguments.clear();
    request.function_name = "shell".into();
    assert!(tool_payload(&request).is_err());
    request.function_name = "sts2_lookup_query".into();
    request.namespace = Some("foreign".into());
    assert!(tool_payload(&request).is_err());
    request.namespace = None;
    request
        .arguments
        .insert("data".into(), json!("x".repeat(wire::TOOL_BYTES)));
    assert!(tool_payload(&request).is_err());
    Ok(())
}

#[test]
fn manifest_tools_require_explicit_profile() -> Result<(), Box<dyn std::error::Error>> {
    for (name, arguments) in [
        (
            "sts2_lookup_content_manifest",
            json!({"operation_id":"manifest-1"}),
        ),
        (
            "sts2_lookup_manifest_read",
            json!({"record_ordinal":31,"offset":131072}),
        ),
    ] {
        let request: ToolRequest = serde_json::from_value(json!({
            "function_name":name,"arguments":arguments
        }))?;
        for profile in [
            LookupProfile::Terminal,
            LookupProfile::Bootstrap,
            LookupProfile::History,
        ] {
            assert!(tool_payload_with_profile(&request, profile).is_err());
        }
        let payload = tool_payload_with_profile(&request, LookupProfile::ContentManifest)?;
        assert_eq!(wire::version_for_payload(&payload), wire::MANIFEST_VERSION);
    }
    assert!(LookupProfile::ContentManifest.admits_bootstrap());
    assert!(LookupProfile::ContentManifest.admits_history());
    Ok(())
}

#[test]
fn additive_profiles_admit_only_their_own_surface() -> Result<(), Box<dyn std::error::Error>> {
    let mut request: ToolRequest = serde_json::from_value(json!({
        "function_name":"sts2_lookup_history",
        "arguments":{"operation":"summary","operation_id":"history-1","branch_id":"branch_root"}
    }))?;
    assert!(tool_payload_with_profile(&request, LookupProfile::Terminal).is_err());
    assert!(tool_payload_with_profile(&request, LookupProfile::Bootstrap).is_err());
    let payload = tool_payload_with_profile(&request, LookupProfile::History)?;
    assert!(matches!(payload, Payload::History { .. }));
    assert_eq!(wire::version_for_payload(&payload), wire::HISTORY_VERSION);

    // Selecting history stays additive: the bootstrap tool the shipped profile had is intact.
    request.function_name = "sts2_lookup_bootstrap".into();
    request.arguments = serde_json::from_value(json!({
        "operation_id":"bootstrap-1",
        "definition_ref":{"content_manifest_id":"content-1","entity_kind":"card",
            "namespaced_id":"ironclad:strike","variant":null},
        "instance_ref":null
    }))?;
    assert!(tool_payload_with_profile(&request, LookupProfile::Terminal).is_err());
    assert!(tool_payload_with_profile(&request, LookupProfile::Bootstrap).is_ok());
    assert!(tool_payload_with_profile(&request, LookupProfile::History).is_ok());
    Ok(())
}

#[test]
fn terminal_requires_paired_owned_tool_events_and_truthful_guard()
-> Result<(), Box<dyn std::error::Error>> {
    let event = |data: Value| {
        serde_json::from_value::<exoharness::Event>(json!({
            "id":"32345678-1234-4234-8234-123456789abc","thread_id":"42345678-1234-4234-8234-123456789abc",
            "session_id":"12345678-1234-4234-8234-123456789abc",
            "turn_id":"22345678-1234-4234-8234-123456789abc","created_at":"2026-09-15T00:00:00Z","data":data
        }))
    };
    let mut events = vec![
        event(
            json!({"type":"tool_requested","tool_call_id":"call_synthetic",
            "request":{"function_name":"sts2_lookup_read","arguments":{"record_ordinal":0,"offset":0}}}),
        )?,
        event(
            json!({"type":"tool_result","tool_call_id":"call_synthetic","result":{"value":"synthetic"}}),
        )?,
        event(
            json!({"type":"messages","messages":[{"role":"assistant","content":"{\"action_id\":\"action-1\"}"}]}),
        )?,
        event(
            json!({"type":"custom","event_type":"sts2.exo-lookup-fetch-guard-v1",
            "payload":{"attempts":2,"forwarded":2,"denied":0,"tools":1}}),
        )?,
        event(json!({"type":"turn_ended"}))?,
    ];
    let result = executor::SendResult {
        session_id: events[0].session_id.ok_or("synthetic session")?,
        turn_id: events[0].turn_id.ok_or("synthetic turn")?,
        latest_event_id: events[0].id,
    };
    assert_eq!(
        crate::lookup_turn::validate_events(&events, &result, 1)?,
        "action-1"
    );
    assert!(crate::lookup_turn::validate_events(&events, &result, 0).is_err());
    events.remove(1);
    assert!(crate::lookup_turn::validate_events(&events, &result, 1).is_err());
    Ok(())
}
