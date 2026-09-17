// SPDX-License-Identifier: MIT
use crate::{Invocation, lookup_runtime::LookupRuntime, lookup_wire as wire, turn};
use executor::{CreateConversationRequest, Harness, SendRequest, SendResult, TypeScriptHarness};
use exoharness::{
    BasicExoHarness, BasicExoHarnessConfig, Event, EventData, EventQuery, EventQueryDirection,
    ExoHarness, SandboxBackendRegistration, SandboxProvider, SecretBackendChoice,
};
use lingua::{Message, universal::UserContent};
use serde::Deserialize;
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use tokio::io::BufReader;

pub async fn run(bootstrap_profile: bool) -> Result<(), &'static str> {
    let mut reader = BufReader::new(tokio::io::stdin());
    let bytes = tokio::time::timeout(Duration::from_secs(5), wire::read_line(&mut reader))
        .await
        .map_err(|_| "exo_lookup_input_timeout")??;
    if bytes.len() > crate::INPUT_LIMIT as usize {
        return Err("exo_lookup_input_bound");
    }
    let invocation: Invocation = wire::decode(&bytes)?;
    if invocation.version != "sts2.exo-lookup-executor-input-v1"
        || !wire::valid_id(&invocation.request_id)
        || !wire::valid_id(&invocation.host_turn_id)
        || invocation.timeout_millis == 0
        || invocation.timeout_millis > 120_000
        || invocation.max_output_tokens == 0
        || invocation.max_output_tokens > 4096
        || !invocation.input.is_object()
    {
        return Err("exo_lookup_invocation");
    }
    let timeout = Duration::from_millis(u64::from(invocation.timeout_millis));
    let relay = Arc::new(LookupRuntime::new(
        invocation.request_id.clone(),
        invocation.host_turn_id.clone(),
        reader,
        bootstrap_profile,
    ));
    tokio::time::timeout(timeout, execute(invocation, relay))
        .await
        .map_err(|_| "exo_lookup_turn_timeout")?
}

async fn execute(
    mut invocation: Invocation,
    relay: Arc<LookupRuntime>,
) -> Result<(), &'static str> {
    let mut key = [0; 32];
    getrandom::fill(&mut key).map_err(|_| "exo_lookup_entropy")?;
    let root = Arc::new(
        BasicExoHarness::new(BasicExoHarnessConfig {
            root: invocation.state_root.clone(),
            secret_backend: SecretBackendChoice::Static(key),
            sandbox_default: SandboxProvider::LocalProcess,
            sandbox_backends: vec![SandboxBackendRegistration::local_process()],
        })
        .await
        .map_err(|_| "exo_lookup_state")?,
    );
    turn::bind_model(
        root.as_ref(),
        &invocation.model,
        &invocation.endpoint,
        std::mem::take(&mut invocation.credential),
    )
    .await
    .map_err(|_| "exo_lookup_binding")?;
    let harness = TypeScriptHarness::new(
        root as Arc<dyn ExoHarness>,
        invocation.source_root.clone(),
        relay.clone(),
    );
    let mut request = turn::agent_request(
        &invocation.module_path,
        &invocation.model,
        invocation.max_output_tokens,
    )
    .map_err(|_| "exo_lookup_module")?;
    request.max_tool_round_trips = Some(32);
    let agent = harness
        .create_agent(request)
        .await
        .map_err(|_| "exo_lookup_agent")?;
    let conversation = agent
        .create_conversation(CreateConversationRequest::default())
        .await
        .map_err(|_| "exo_lookup_conversation")?;
    let result = conversation
        .send(SendRequest {
            input: vec![Message::User {
                content: UserContent::String(
                    serde_json::to_string(&invocation.input).map_err(|_| "exo_lookup_input")?,
                ),
            }],
            session_id: None,
        })
        .await
        .map_err(|_| "exo_lookup_send")?;
    let events = conversation
        .exoharness_handle()
        .get_events(Some(EventQuery {
            direction: Some(EventQueryDirection::Asc),
            limit: Some(257),
            turn_id: Some(result.turn_id),
            session_id: Some(result.session_id),
            ..EventQuery::default()
        }))
        .await
        .map_err(|_| "exo_lookup_events")?;
    let tools = relay.count().await?;
    let action = validate_events(&events.events, &result, tools)?;
    if !invocation.input["legal_action_ids"]
        .as_array()
        .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(action.as_str())))
    {
        return Err("exo_lookup_action");
    }
    relay.decision(action).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuardCounts {
    attempts: u64,
    forwarded: u64,
    denied: u64,
    tools: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Action {
    action_id: String,
}

pub(super) fn validate_events(
    events: &[Event],
    result: &SendResult,
    executed: u64,
) -> Result<String, &'static str> {
    if events.is_empty() || events.len() > 256 || executed > 32 {
        return Err("exo_lookup_event_bound");
    }
    let mut guards = 0;
    let mut pending = BTreeSet::new();
    let mut requested = BTreeSet::new();
    let mut texts = Vec::new();
    let mut ended = false;
    for event in events {
        if event.turn_id != Some(result.turn_id) || event.session_id != Some(result.session_id) {
            return Err("exo_lookup_event_identity");
        }
        // Bound the whole retained event stream, not only the final action text.
        if serde_json::to_vec(event)
            .map_err(|_| "exo_lookup_event")?
            .len()
            > wire::FRAME_BYTES
        {
            return Err("exo_lookup_event_bound");
        }
        match &event.data {
            EventData::Error { .. } => return Err("exo_lookup_event_error"),
            EventData::TurnEnded => {
                if ended || !pending.is_empty() {
                    return Err("exo_lookup_terminal");
                }
                ended = true;
            }
            EventData::Custom {
                event_type,
                payload,
            } if event_type == "sts2.exo-lookup-fetch-guard-v1" => {
                guards += 1;
                let counts: GuardCounts =
                    serde_json::from_value(payload.clone()).map_err(|_| "exo_lookup_guard")?;
                if counts.attempts != counts.forwarded
                    || counts.denied != 0
                    || counts.tools != executed
                    || !(1..=33).contains(&counts.forwarded)
                    || counts.forwarded > executed + 1
                {
                    return Err("exo_lookup_guard");
                }
            }
            EventData::ToolRequested {
                tool_call_id,
                request,
                ..
            } => {
                if ended
                    || request.namespace.is_some()
                    || !matches!(
                        request.function_name.as_str(),
                        "sts2_lookup_query" | "sts2_lookup_bootstrap" | "sts2_lookup_read"
                    )
                    || serde_json::to_vec(&request.arguments)
                        .map_err(|_| "exo_lookup_event")?
                        .len()
                        > wire::TOOL_BYTES
                    || !requested.insert(tool_call_id.to_string())
                {
                    return Err("exo_lookup_tool_event");
                }
                pending.insert(tool_call_id.to_string());
                texts.clear();
            }
            EventData::ToolResult { tool_call_id, .. } => {
                if ended || !pending.remove(&tool_call_id.to_string()) {
                    return Err("exo_lookup_tool_event");
                }
            }
            EventData::Messages { messages, .. } => {
                if ended {
                    return Err("exo_lookup_terminal");
                }
                for message in messages {
                    if let Message::Assistant { content, .. } = message {
                        if !pending.is_empty() {
                            return Err("exo_lookup_terminal");
                        }
                        // Earlier model messages may contain tool-call parts. Only the final
                        // post-tool assistant message may become a closed terminal action.
                        texts.push(turn::assistant_text(content.clone()));
                    }
                }
            }
            _ => {}
        }
    }
    if !ended
        || guards != 1
        || !pending.is_empty()
        || requested.len() as u64 != executed
        || texts.len() != 1
    {
        return Err("exo_lookup_terminal");
    }
    let text = texts
        .pop()
        .ok_or("exo_lookup_terminal")?
        .map_err(|_| "exo_lookup_terminal")?;
    if text.len() > 8192 {
        return Err("exo_lookup_terminal_bound");
    }
    let action: Action = wire::decode(text.as_bytes())?;
    if action.action_id.is_empty() || action.action_id.len() > 128 {
        return Err("exo_lookup_action");
    }
    Ok(action.action_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    fn result() -> Result<SendResult, serde_json::Error> {
        Ok(SendResult {
            session_id: serde_json::from_value(json!("12345678-1234-4234-8234-123456789abc"))?,
            turn_id: serde_json::from_value(json!("22345678-1234-4234-8234-123456789abc"))?,
            latest_event_id: serde_json::from_value(json!("52345678-1234-4234-8234-123456789abc"))?,
        })
    }
    fn event(data: Value) -> Result<Event, serde_json::Error> {
        serde_json::from_value(json!({"id":"32345678-1234-4234-8234-123456789abc",
            "thread_id":"42345678-1234-4234-8234-123456789abc",
            "session_id":"12345678-1234-4234-8234-123456789abc",
            "turn_id":"22345678-1234-4234-8234-123456789abc",
            "created_at":"2026-09-15T00:00:00Z","data":data}))
    }
    #[test]
    fn closed_terminal_requires_correlated_guard_and_completed_turn()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut events = vec![
            event(
                json!({"type":"messages","messages":[{"role":"assistant","content":"{\"action_id\":\"action-1\"}"}]}),
            )?,
            event(
                json!({"type":"custom","event_type":"sts2.exo-lookup-fetch-guard-v1",
                "payload":{"attempts":1,"forwarded":1,"denied":0,"tools":0}}),
            )?,
            event(json!({"type":"turn_ended"}))?,
        ];
        assert_eq!(validate_events(&events, &result()?, 0)?, "action-1");
        events.pop();
        assert!(validate_events(&events, &result()?, 0).is_err());
        events.push(event(json!({"type":"turn_ended"}))?);
        events.push(event(
            json!({"type":"error","message":"untrusted producer text"}),
        )?);
        assert!(validate_events(&events, &result()?, 0).is_err());
        assert!(wire::decode::<Action>(br#"{"action_id":"a","rationale":"extra"}"#).is_err());
        assert!(wire::decode::<Action>(br#"{"action_id":"a","action_id":"b"}"#).is_err());
        Ok(())
    }
}
