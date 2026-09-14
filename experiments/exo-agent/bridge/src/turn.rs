// SPDX-License-Identifier: MIT

use executor::{
    AgentHarnessKind, BasicToolRuntime, CreateAgentRequest, CreateConversationRequest, Harness,
    SendRequest, SendResult, TypeScriptHarness, TypeScriptHarnessConfig,
};
use exoharness::{
    BasicExoHarness, BasicExoHarnessConfig, Binding, Event, EventData, EventQuery,
    EventQueryDirection, ExoHarness, PutSecretRequest, SandboxBackendRegistration, SandboxProvider,
    Secret, SecretBackendChoice,
};
use lingua::Message;
use lingua::universal::{AssistantContent, AssistantContentPart, UserContent};
use std::sync::Arc;

use crate::{Invocation, Receipt};

async fn create_harness(
    invocation: &mut Invocation,
) -> Result<TypeScriptHarness<BasicToolRuntime>, &'static str> {
    let mut encryption_key = [0_u8; 32];
    getrandom::fill(&mut encryption_key).map_err(|_| "exo_executor_entropy")?;
    let root = Arc::new(
        BasicExoHarness::new(BasicExoHarnessConfig {
            root: invocation.state_root.clone(),
            secret_backend: SecretBackendChoice::Static(encryption_key),
            sandbox_default: SandboxProvider::LocalProcess,
            // Required by upstream's configuration validator; no model tool is registered.
            sandbox_backends: vec![SandboxBackendRegistration::local_process()],
        })
        .await
        .map_err(|_| "exo_executor_state")?,
    );
    bind_model(
        root.as_ref(),
        &invocation.model,
        &invocation.endpoint,
        std::mem::take(&mut invocation.credential),
    )
    .await
    .map_err(|_| "exo_executor_binding")?;
    Ok(TypeScriptHarness::new(
        root as Arc<dyn ExoHarness>,
        invocation.source_root.clone(),
        Arc::new(BasicToolRuntime),
    ))
}

pub(super) async fn execute(mut invocation: Invocation) -> Result<Receipt, &'static str> {
    let harness = create_harness(&mut invocation).await?;
    let agent = harness
        .create_agent(
            agent_request(
                &invocation.module_path,
                &invocation.model,
                invocation.max_output_tokens,
            )
            .map_err(|_| "exo_executor_module")?,
        )
        .await
        .map_err(|_| "exo_executor_agent")?;
    let conversation = agent
        .create_conversation(CreateConversationRequest::default())
        .await
        .map_err(|_| "exo_executor_conversation")?;
    let result = conversation
        .send(SendRequest {
            input: vec![Message::User {
                content: UserContent::String(
                    serde_json::to_string(&invocation.input)
                        .map_err(|_| "exo_executor_projection")?,
                ),
            }],
            session_id: None,
        })
        .await;
    let events = conversation
        .exoharness_handle()
        .get_events(Some(EventQuery {
            direction: Some(EventQueryDirection::Asc),
            limit: Some(65),
            ..EventQuery::default()
        }))
        .await
        .map_err(|_| "exo_executor_events")?;
    receipt(invocation, &events.events, result.ok())
}

fn receipt(
    invocation: Invocation,
    events: &[Event],
    result: Option<SendResult>,
) -> Result<Receipt, &'static str> {
    let (event, attempts, forwarded, denied) = guard_evidence(events)?;
    let turn_id = event.turn_id.ok_or("exo_executor_turn_identity")?;
    let session_id = event.session_id.ok_or("exo_executor_session_identity")?;
    let decision = if result
        .as_ref()
        .is_some_and(|result| result.turn_id == turn_id && result.session_id == session_id)
        && attempts == 1
        && forwarded == 1
        && denied == 0
    {
        terminal(events.iter().map(|event| event.data.clone())).ok()
    } else {
        None
    };
    Ok(Receipt {
        version: "sts2.exo-executor-receipt-v1",
        request_id: invocation.request_id,
        host_turn_id: invocation.host_turn_id,
        exo_turn_id: turn_id.to_string(),
        exo_session_id: session_id.to_string(),
        error_code: decision.is_none().then_some("exo_turn_failed"),
        decision,
        fetch_attempts: attempts,
        forwarded_requests: forwarded,
        denied_requests: denied,
    })
}

fn guard_evidence(events: &[Event]) -> Result<(&Event, u64, u64, u64), &'static str> {
    if events.len() > 64 {
        return Err("exo_executor_events_bound");
    }
    let evidence = events
        .iter()
        .filter(|event| {
            matches!(&event.data, EventData::Custom { event_type, .. }
                if event_type == "sts2.exo-fetch-guard-v1")
        })
        .collect::<Vec<_>>();
    if evidence.len() != 1 {
        return Err("exo_executor_guard_missing");
    }
    let event = evidence[0];
    let EventData::Custom { payload, .. } = &event.data else {
        return Err("exo_executor_guard_shape");
    };
    let attempts = payload
        .get("attempts")
        .and_then(serde_json::Value::as_u64)
        .ok_or("exo_executor_guard_counts")?;
    let forwarded = payload
        .get("forwarded")
        .and_then(serde_json::Value::as_u64)
        .ok_or("exo_executor_guard_counts")?;
    let denied = payload
        .get("denied")
        .and_then(serde_json::Value::as_u64)
        .ok_or("exo_executor_guard_counts")?;
    Ok((event, attempts, forwarded, denied))
}

async fn bind_model(
    root: &dyn ExoHarness,
    model: &str,
    endpoint: &str,
    secret: String,
) -> Result<(), ()> {
    if secret.is_empty() {
        return Err(());
    }
    let secret_id = root
        .put_secret(PutSecretRequest {
            name: String::from("sts2-model"),
            secret: Secret::Key { value: secret },
        })
        .await
        .map_err(|_| ())?;
    root.put_binding(Binding::Llm {
        name: model.to_owned(),
        model: model.to_owned(),
        base_url: Some(endpoint.to_owned()),
        secret_id: Some(secret_id),
    })
    .await
    .map(|_| ())
    .map_err(|_| ())
}

fn agent_request(
    module: &std::path::Path,
    model: &str,
    tokens: u32,
) -> Result<CreateAgentRequest, ()> {
    Ok(CreateAgentRequest {
        slug: String::from("sts2"),
        name: None,
        harness: AgentHarnessKind::TypeScript,
        typescript: Some(TypeScriptHarnessConfig {
            module_path: module.to_str().ok_or(())?.to_owned(),
            tool_module_paths: Vec::new(),
        }),
        enable_agent_tool_creation: false,
        sandbox_image: None,
        sandbox_provider: SandboxProvider::LocalProcess,
        sandbox_scope: None,
        enable_networking: false,
        model: model.to_owned(),
        max_output_tokens: Some(i64::from(tokens)),
        max_tool_round_trips: Some(0),
        braintrust: None,
    })
}

fn terminal(events: impl Iterator<Item = EventData>) -> Result<String, ()> {
    let mut terminal = None;
    let mut ended = false;
    for event in events {
        match event {
            EventData::TurnEnded => ended = true,
            EventData::Error { .. }
            | EventData::ToolRequested { .. }
            | EventData::ToolResult { .. } => {
                return Err(());
            }
            EventData::Messages { messages, .. } => {
                for message in messages {
                    match message {
                        Message::Assistant { content, .. } => {
                            if terminal.is_some() {
                                return Err(());
                            }
                            terminal = Some(assistant_text(content)?);
                        }
                        Message::User { .. } => {}
                        _ => return Err(()),
                    }
                }
            }
            _ => {}
        }
    }
    let decision = terminal.ok_or(())?;
    if !ended || decision.is_empty() || decision.len() > 8192 {
        return Err(());
    }
    Ok(decision)
}

fn assistant_text(content: AssistantContent) -> Result<String, ()> {
    match content {
        AssistantContent::String(text) => Ok(text),
        AssistantContent::Array(parts) => {
            let mut text = String::new();
            for part in parts {
                match part {
                    AssistantContentPart::Text(part) => text.push_str(&part.text),
                    // Reasoning never becomes terminal output or public evidence.
                    AssistantContentPart::Reasoning { .. } => {}
                    _ => return Err(()),
                }
            }
            Ok(text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_requires_one_completed_assistant_message() -> Result<(), serde_json::Error> {
        let messages = || {
            serde_json::from_value::<EventData>(serde_json::json!({
                "type": "messages",
                "messages": [{"role": "assistant", "content": "{\"decision\":\"wait\"}"}]
            }))
        };
        // Use the upstream serialization contract; schema drift fails this test.
        let event = messages()?;
        assert!(terminal([event.clone()].into_iter()).is_err());
        assert_eq!(
            terminal([event.clone(), EventData::TurnEnded].into_iter()),
            Ok(String::from("{\"decision\":\"wait\"}"))
        );
        assert!(terminal([event.clone(), event, EventData::TurnEnded].into_iter()).is_err());
        Ok(())
    }

    #[test]
    fn terminal_rejects_empty_and_oversized_text() -> Result<(), serde_json::Error> {
        for text in [String::new(), "x".repeat(8193)] {
            let event = serde_json::from_value::<EventData>(serde_json::json!({
                "type": "messages", "messages": [{"role": "assistant", "content": text}]
            }))?;
            assert!(terminal([event, EventData::TurnEnded].into_iter()).is_err());
        }
        Ok(())
    }
}
