// SPDX-License-Identifier: MIT

//! Evidence the executor derives from the extension's own custom events and terminal message.

use exoharness::{Event, EventData};
use lingua::Message;
use lingua::universal::{AssistantContent, AssistantContentPart};
use serde_json::Value;

const FETCH_GUARD: &str = "sts2.exo-fetch-guard-v1";
const TOOL_GUARD: &str = "sts2.exo-tool-guard-v1";

/// Returns the fetch-guard event with its attempted, forwarded and denied request counts.
pub(super) fn guard_evidence(events: &[Event]) -> Result<(&Event, u64, u64, u64), &'static str> {
    if events.len() > 64 {
        return Err("exo_executor_events_bound");
    }
    let (event, payload) = custom_payload(
        events,
        FETCH_GUARD,
        "exo_executor_guard_missing",
        "exo_executor_guard_shape",
    )?;
    let count = |key: &str| {
        payload
            .get(key)
            .and_then(Value::as_u64)
            .ok_or("exo_executor_guard_counts")
    };
    Ok((
        event,
        count("attempts")?,
        count("forwarded")?,
        count("denied")?,
    ))
}

/// Returns how many tool registrations and dispatches the extension denied with the typed
/// `sts2_forbidden_tool` error. The event is required: an extension that does not seal the
/// actual registry is not the reviewed tool-free extension, and its turn cannot be trusted.
pub(super) fn forbidden_tool_denials(events: &[Event]) -> Result<u64, &'static str> {
    let (_, payload) = custom_payload(
        events,
        TOOL_GUARD,
        "exo_executor_tool_guard_missing",
        "exo_executor_tool_guard_shape",
    )?;
    let count = |key: &str| {
        payload
            .get(key)
            .and_then(Value::as_u64)
            .ok_or("exo_executor_tool_guard_counts")
    };
    count("registrations")?
        .checked_add(count("dispatches")?)
        .ok_or("exo_executor_tool_guard_counts")
}

fn custom_payload<'a>(
    events: &'a [Event],
    expected: &str,
    missing: &'static str,
    shape: &'static str,
) -> Result<(&'a Event, &'a Value), &'static str> {
    let mut found = events.iter().filter(|event| {
        matches!(&event.data, EventData::Custom { event_type, .. } if event_type == expected)
    });
    let event = found.next().ok_or(missing)?;
    if found.next().is_some() {
        return Err(missing);
    }
    match &event.data {
        EventData::Custom { payload, .. } if payload.is_object() => Ok((event, payload)),
        _ => Err(shape),
    }
}

pub(super) fn terminal(events: impl Iterator<Item = EventData>) -> Result<String, ()> {
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

pub(super) fn assistant_text(content: AssistantContent) -> Result<String, ()> {
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
    use serde_json::json;

    fn event(data: Value) -> Result<Event, serde_json::Error> {
        // Use the upstream serialization contract; schema drift fails these tests.
        serde_json::from_value(json!({
            "id": "01a0a70b-bff4-7dc1-8073-ff55f2fb704e",
            "thread_id": "01a0a70b-bff4-7dc1-8073-ff55f2fb704f",
            "session_id": "01a0a70b-bff4-7dc1-8073-ff55f2fb7050",
            "turn_id": "01a0a70b-bff4-7dc1-8073-ff55f2fb7051",
            "created_at": "2026-09-17T00:00:00Z",
            "data": data
        }))
    }

    fn custom(event_type: &str, payload: Value) -> Result<Event, serde_json::Error> {
        event(json!({"type": "custom", "event_type": event_type, "payload": payload}))
    }

    fn message(text: &str) -> Result<EventData, serde_json::Error> {
        serde_json::from_value(json!({
            "type": "messages",
            "messages": [{"role": "assistant", "content": text}]
        }))
    }

    #[test]
    fn terminal_requires_one_completed_assistant_message() -> Result<(), serde_json::Error> {
        let event = message("{\"decision\":\"wait\"}")?;
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
            let event = message(&text)?;
            assert!(terminal([event, EventData::TurnEnded].into_iter()).is_err());
        }
        Ok(())
    }

    #[test]
    fn tool_guard_event_is_required_and_its_denials_are_summed() -> Result<(), serde_json::Error> {
        let fetch = custom(
            FETCH_GUARD,
            json!({"attempts": 1, "forwarded": 1, "denied": 0}),
        )?;
        let sealed = custom(TOOL_GUARD, json!({"registrations": 0, "dispatches": 0}))?;
        let denied = custom(TOOL_GUARD, json!({"registrations": 1, "dispatches": 2}))?;
        assert_eq!(
            forbidden_tool_denials(&[fetch.clone(), sealed.clone()]),
            Ok(0)
        );
        assert_eq!(
            forbidden_tool_denials(&[fetch.clone(), denied.clone()]),
            Ok(3)
        );
        assert_eq!(
            forbidden_tool_denials(std::slice::from_ref(&fetch)),
            Err("exo_executor_tool_guard_missing")
        );
        assert_eq!(
            forbidden_tool_denials(&[fetch.clone(), sealed.clone(), denied]),
            Err("exo_executor_tool_guard_missing")
        );
        assert_eq!(
            forbidden_tool_denials(&[
                fetch.clone(),
                custom(TOOL_GUARD, json!({"registrations": 1}))?
            ]),
            Err("exo_executor_tool_guard_counts")
        );
        assert_eq!(
            forbidden_tool_denials(&[fetch, custom(TOOL_GUARD, json!("sealed"))?]),
            Err("exo_executor_tool_guard_shape")
        );
        assert_eq!(
            guard_evidence(&[sealed]).err(),
            Some("exo_executor_guard_missing")
        );
        Ok(())
    }

    #[test]
    fn forbidden_tool_yields_the_typed_code_and_never_a_decision() -> Result<(), serde_json::Error>
    {
        let invocation = || crate::Invocation {
            version: String::from("sts2.exo-executor-input-v1"),
            request_id: String::from("request"),
            host_turn_id: String::from("turn"),
            model: String::from("o3-pro"),
            endpoint: String::from("http://127.0.0.1:1"),
            module_path: "module".into(),
            source_root: "source".into(),
            state_root: "state".into(),
            input: json!({}),
            timeout_millis: 1,
            max_output_tokens: 1,
            credential: String::new(),
        };
        let fetch = custom(
            FETCH_GUARD,
            json!({"attempts": 1, "forwarded": 1, "denied": 0}),
        )?;
        let decision = event(serde_json::to_value(message("{\"decision\":\"wait\"}")?)?)?;
        let ended = event(json!({"type": "turn_ended"}))?;
        let denied = custom(TOOL_GUARD, json!({"registrations": 0, "dispatches": 1}))?;
        let receipt = crate::turn::receipt(
            invocation(),
            "agent",
            "conversation",
            &[fetch.clone(), decision.clone(), ended.clone(), denied],
            None,
        );
        let receipt = receipt.map_err(serde::de::Error::custom)?;
        assert_eq!(receipt.error_code, Some("exo_forbidden_tool"));
        assert!(receipt.decision.is_none());
        assert_eq!((receipt.fetch_attempts, receipt.forwarded_requests), (1, 1));
        let sealed = custom(TOOL_GUARD, json!({"registrations": 0, "dispatches": 0}))?;
        let receipt = crate::turn::receipt(
            invocation(),
            "agent",
            "conversation",
            &[fetch, decision, ended, sealed],
            None,
        )
        .map_err(serde::de::Error::custom)?;
        assert_eq!(receipt.error_code, Some("exo_turn_failed"));
        Ok(())
    }
}
