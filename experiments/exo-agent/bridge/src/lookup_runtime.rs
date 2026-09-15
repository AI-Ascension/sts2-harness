// SPDX-License-Identifier: MIT
use crate::lookup_wire::{self as wire, Frame, Payload};
use async_trait::async_trait;
use executor::{AgentConfig, ConversationConfig, ToolRuntime};
use exoharness::{AgentHandle, ConversationHandle, ToolRequest, ToolResult, TurnHandle};
use serde::Deserialize;
use serde_json::Value;
use tokio::{
    io::{BufReader, Stdin, Stdout},
    sync::Mutex,
};

pub struct LookupRuntime {
    request_id: String,
    turn_id: String,
    channel: Mutex<Channel>,
}
struct Channel {
    input: BufReader<Stdin>,
    output: Stdout,
    tools: u64,
    failed: bool,
}

impl LookupRuntime {
    pub fn new(request_id: String, turn_id: String, input: BufReader<Stdin>) -> Self {
        Self {
            request_id,
            turn_id,
            channel: Mutex::new(Channel {
                input,
                output: tokio::io::stdout(),
                tools: 0,
                failed: false,
            }),
        }
    }
    pub async fn count(&self) -> Result<u64, &'static str> {
        let channel = self.channel.lock().await;
        if channel.failed {
            return Err("exo_lookup_runtime_failed");
        }
        Ok(channel.tools)
    }
    pub async fn decision(&self, action_id: String) -> Result<(), &'static str> {
        let mut channel = self.channel.lock().await;
        if channel.failed {
            return Err("exo_lookup_runtime_failed");
        }
        let frame = Frame {
            wire_version: wire::VERSION.into(),
            request_id: self.request_id.clone(),
            turn_id: self.turn_id.clone(),
            sequence: channel.tools + 1,
            payload: Payload::Decision { action_id },
        };
        channel.failed = true; // No second terminal or tool may follow publication.
        wire::write_frame(&mut channel.output, &frame).await
    }
    async fn relay(&self, request: &ToolRequest) -> Result<Value, &'static str> {
        let mut channel = self.channel.lock().await;
        if channel.failed || channel.tools >= 32 {
            channel.failed = true;
            return Err("exo_lookup_tool_bound");
        }
        channel.failed = true; // Remains failed on every validation, I/O or cancellation exit.
        let payload = tool_payload(request)?;
        channel.tools += 1;
        let sequence = channel.tools;
        let frame = Frame {
            wire_version: wire::VERSION.into(),
            request_id: self.request_id.clone(),
            turn_id: self.turn_id.clone(),
            sequence,
            payload,
        };
        wire::write_frame(&mut channel.output, &frame).await?;
        let bytes = wire::read_line(&mut channel.input).await?;
        let value = wire::feedback(&bytes, &self.request_id, &self.turn_id, sequence)?;
        channel.failed = false;
        Ok(value)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArguments {
    record_ordinal: usize,
    offset: usize,
}

fn tool_payload(request: &ToolRequest) -> Result<Payload, &'static str> {
    if request.namespace.is_some()
        || serde_json::to_vec(&request.arguments)
            .map_err(|_| "exo_lookup_arguments")?
            .len()
            > wire::TOOL_BYTES
    {
        return Err("exo_lookup_arguments");
    }
    let arguments = Value::Object(request.arguments.clone());
    match request.function_name.as_str() {
        "sts2_lookup_query" => Ok(Payload::Query { arguments }),
        "sts2_lookup_read" => {
            let read: ReadArguments =
                serde_json::from_value(arguments).map_err(|_| "exo_lookup_arguments")?;
            if read.record_ordinal > 255 || read.offset > 65536 {
                return Err("exo_lookup_arguments");
            }
            Ok(Payload::ReadRetained {
                record_ordinal: read.record_ordinal,
                offset: read.offset,
            })
        }
        _ => Err("exo_lookup_tool_denied"),
    }
}

#[async_trait]
impl ToolRuntime for LookupRuntime {
    async fn execute(
        &self,
        _agent: &dyn AgentHandle,
        _conversation: &dyn ConversationHandle,
        turn: Option<&dyn TurnHandle>,
        _agent_config: &AgentConfig,
        _config: &ConversationConfig,
        request: &ToolRequest,
    ) -> exoharness::Result<ToolResult> {
        if turn.is_none() {
            self.channel.lock().await.failed = true;
            return Err(anyhow::anyhow!("exo_lookup_turn_missing"));
        }
        self.relay(request)
            .await
            .map_err(|code| anyhow::anyhow!(code))
    }
}

#[cfg(test)]
mod tests {
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
}
