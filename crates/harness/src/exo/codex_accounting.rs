// SPDX-License-Identifier: MIT

//! Bounded metadata extracted from the Codex `exec --json` event stream.
//!
//! This module intentionally keeps no event payload, prompt, rationale, or model message. It
//! records only the provider thread identity, bounded event counters, and token usage fields that
//! Codex explicitly reports. The bridge may persist the resulting summary in an operator-owned
//! sidecar; the harness trajectory does not receive this stream.

const MAX_EVENTS: usize = 4_096;
const MAX_EVENT_LINE_BYTES: usize = 16 * 1024;
const MAX_STREAM_BYTES: usize = 128 * 1024;
const MAX_ID_BYTES: usize = 512;
const MAX_TOKEN_COUNT: u64 = 1_000_000_000_000;

/// Token counts reported by one completed Codex turn. Optional fields remain absent when the
/// installed CLI does not expose them, so a missing field is never misreported as zero.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexTokenUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_input_tokens: Option<u64>,
    pub output_tokens: u64,
    pub reasoning_output_tokens: Option<u64>,
}

/// Whether a valid `turn.completed.usage` object was observed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexUsageStatus {
    Reported,
    Unavailable,
}

impl CodexUsageStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reported => "reported",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Status of the structured event stream, without retaining its untrusted contents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexStreamStatus {
    Complete,
    Empty,
    Invalid,
}

impl CodexStreamStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Empty => "empty",
            Self::Invalid => "invalid",
        }
    }
}

/// Sanitized accounting summary for one bridge invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexEventAccounting {
    pub provider_request_id: Option<String>,
    pub event_count: usize,
    pub turn_count: usize,
    pub completed_turn_count: usize,
    pub usage_status: CodexUsageStatus,
    pub usage: Option<CodexTokenUsage>,
    pub stream_status: CodexStreamStatus,
}

/// Structured event parsing failures. Error text is for local diagnostics only and must not be
/// copied into a persisted provider record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexEventError {
    StreamTooLarge,
    EventLineTooLarge,
    TooManyEvents,
    InvalidUtf8,
    MalformedJson,
    InvalidEvent,
    InvalidUsage,
}

impl std::fmt::Display for CodexEventError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::StreamTooLarge => "Codex event stream exceeds its byte bound",
            Self::EventLineTooLarge => "Codex event line exceeds its byte bound",
            Self::TooManyEvents => "Codex event stream exceeds its event bound",
            Self::InvalidUtf8 => "Codex event stream is not UTF-8",
            Self::MalformedJson => "Codex event is not valid JSON",
            Self::InvalidEvent => "Codex event has an invalid structured shape",
            Self::InvalidUsage => "Codex usage object has an invalid shape",
        })
    }
}

impl std::error::Error for CodexEventError {}

/// Parses only the metadata-bearing parts of Codex JSONL events.
///
/// Unknown event types are ignored for forward compatibility. Known event fields are validated;
/// any malformed or oversized event fails closed before it can be treated as reported usage.
pub fn parse_codex_events(stream: &[u8]) -> Result<CodexEventAccounting, CodexEventError> {
    if stream.len() > MAX_STREAM_BYTES {
        return Err(CodexEventError::StreamTooLarge);
    }
    if stream.is_empty() {
        return Ok(CodexEventAccounting {
            provider_request_id: None,
            event_count: 0,
            turn_count: 0,
            completed_turn_count: 0,
            usage_status: CodexUsageStatus::Unavailable,
            usage: None,
            stream_status: CodexStreamStatus::Empty,
        });
    }

    let mut provider_request_id = None;
    let mut event_count = 0_usize;
    let mut turn_count = 0_usize;
    let mut completed_turn_count = 0_usize;
    let mut usage = None;
    for line in stream.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        if line.len() > MAX_EVENT_LINE_BYTES {
            return Err(CodexEventError::EventLineTooLarge);
        }
        event_count = event_count
            .checked_add(1)
            .filter(|count| *count <= MAX_EVENTS)
            .ok_or(CodexEventError::TooManyEvents)?;
        let line = std::str::from_utf8(line).map_err(|_| CodexEventError::InvalidUtf8)?;
        let event: serde_json::Value =
            serde_json::from_str(line).map_err(|_| CodexEventError::MalformedJson)?;
        let object = event.as_object().ok_or(CodexEventError::InvalidEvent)?;
        let event_type = object
            .get("type")
            .and_then(serde_json::Value::as_str)
            .ok_or(CodexEventError::InvalidEvent)?;
        match event_type {
            "thread.started" => {
                if let Some(thread_id) = object.get("thread_id") {
                    let value = thread_id
                        .as_str()
                        .filter(|value| valid_identity(value))
                        .ok_or(CodexEventError::InvalidEvent)?;
                    if provider_request_id
                        .as_deref()
                        .is_some_and(|existing| existing != value)
                    {
                        return Err(CodexEventError::InvalidEvent);
                    }
                    provider_request_id = Some(value.to_owned());
                }
            }
            "turn.started" => {
                turn_count = turn_count
                    .checked_add(1)
                    .filter(|count| *count <= MAX_EVENTS)
                    .ok_or(CodexEventError::TooManyEvents)?;
            }
            "turn.completed" => {
                completed_turn_count = completed_turn_count
                    .checked_add(1)
                    .filter(|count| *count <= MAX_EVENTS)
                    .ok_or(CodexEventError::TooManyEvents)?;
                usage = match object.get("usage") {
                    Some(value) if !value.is_null() => Some(parse_usage(value)?),
                    _ => None,
                };
            }
            _ => {}
        }
    }

    Ok(CodexEventAccounting {
        provider_request_id,
        event_count,
        turn_count,
        completed_turn_count,
        usage_status: if usage.is_some() {
            CodexUsageStatus::Reported
        } else {
            CodexUsageStatus::Unavailable
        },
        usage,
        stream_status: CodexStreamStatus::Complete,
    })
}

fn parse_usage(value: &serde_json::Value) -> Result<CodexTokenUsage, CodexEventError> {
    let object = value.as_object().ok_or(CodexEventError::InvalidUsage)?;
    Ok(CodexTokenUsage {
        input_tokens: required_count(object, "input_tokens")?,
        cached_input_tokens: required_count(object, "cached_input_tokens")?,
        cache_write_input_tokens: optional_count(object, "cache_write_input_tokens")?,
        output_tokens: required_count(object, "output_tokens")?,
        reasoning_output_tokens: optional_count(object, "reasoning_output_tokens")?,
    })
}

fn required_count(
    object: &serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Result<u64, CodexEventError> {
    object
        .get(name)
        .and_then(serde_json::Value::as_u64)
        .filter(|value| *value <= MAX_TOKEN_COUNT)
        .ok_or(CodexEventError::InvalidUsage)
}

fn optional_count(
    object: &serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Result<Option<u64>, CodexEventError> {
    match object.get(name) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .filter(|value| *value <= MAX_TOKEN_COUNT)
            .map(Some)
            .ok_or(CodexEventError::InvalidUsage),
    }
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_thread_and_complete_usage_without_retaining_event_text()
    -> Result<(), Box<dyn std::error::Error>> {
        let stream = br#"{"type":"thread.started","thread_id":"thread-1"}
{"type":"item.completed","item":{"type":"agent_message","text":"private decision"}}
{"type":"turn.completed","usage":{"input_tokens":31,"cached_input_tokens":7,"output_tokens":11,"reasoning_output_tokens":3}}"#;
        let parsed = parse_codex_events(stream)?;
        assert_eq!(parsed.provider_request_id.as_deref(), Some("thread-1"));
        assert_eq!(parsed.event_count, 3);
        assert_eq!(parsed.completed_turn_count, 1);
        assert_eq!(parsed.usage_status, CodexUsageStatus::Reported);
        assert_eq!(
            parsed
                .usage
                .as_ref()
                .ok_or("usage is absent")?
                .output_tokens,
            11
        );
        assert!(!format!("{parsed:?}").contains("private decision"));
        Ok(())
    }

    #[test]
    fn omitted_usage_is_explicitly_unavailable() -> Result<(), Box<dyn std::error::Error>> {
        let parsed = parse_codex_events(
            br#"{"type":"thread.started","thread_id":"thread-1"}
{"type":"turn.started"}
{"type":"turn.completed"}"#,
        )?;
        assert_eq!(parsed.usage_status, CodexUsageStatus::Unavailable);
        assert!(parsed.usage.is_none());
        Ok(())
    }

    #[test]
    fn malformed_usage_and_event_bounds_fail_closed() {
        assert_eq!(
            parse_codex_events(br#"{"type":"turn.completed","usage":{"output_tokens":1}}"#),
            Err(CodexEventError::InvalidUsage)
        );
        assert_eq!(
            parse_codex_events(&vec![b'x'; MAX_EVENT_LINE_BYTES + 1]),
            Err(CodexEventError::EventLineTooLarge)
        );
    }
}
