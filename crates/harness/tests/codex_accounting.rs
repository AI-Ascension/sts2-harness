// SPDX-License-Identifier: MIT

use sts2_harness::{CodexEventError, CodexStreamStatus, CodexUsageStatus, parse_codex_events};

#[test]
fn structured_events_capture_usage_and_thread_identity_only()
-> Result<(), Box<dyn std::error::Error>> {
    let events = br#"{"type":"thread.started","thread_id":"codex-thread-7"}
{"type":"item.completed","item":{"type":"agent_message","text":"private rationale"}}
{"type":"turn.started"}
{"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":40,"cache_write_input_tokens":2,"output_tokens":17,"reasoning_output_tokens":9}}"#;
    let accounting = parse_codex_events(events)?;
    assert_eq!(
        accounting.provider_request_id.as_deref(),
        Some("codex-thread-7")
    );
    assert_eq!(accounting.usage_status, CodexUsageStatus::Reported);
    assert_eq!(accounting.stream_status, CodexStreamStatus::Complete);
    assert_eq!(
        accounting
            .usage
            .as_ref()
            .ok_or("usage is absent")?
            .input_tokens,
        100
    );
    assert!(!format!("{accounting:?}").contains("private rationale"));
    Ok(())
}

#[test]
fn missing_provider_usage_is_explicit_and_does_not_become_zero()
-> Result<(), Box<dyn std::error::Error>> {
    let accounting = parse_codex_events(
        br#"{"type":"thread.started","thread_id":"codex-thread-8"}
{"type":"turn.completed"}"#,
    )?;
    assert_eq!(accounting.usage_status, CodexUsageStatus::Unavailable);
    assert!(accounting.usage.is_none());
    Ok(())
}

#[test]
fn malformed_structured_usage_and_provider_identity_fail_closed() {
    assert_eq!(
        parse_codex_events(
            br#"{"type":"thread.started","thread_id":"codex-thread-1"}
{"type":"thread.started","thread_id":"codex-thread-2"}"#,
        ),
        Err(CodexEventError::InvalidEvent)
    );
    assert_eq!(
        parse_codex_events(br#"{"type":"turn.completed","usage":{"input_tokens":1,"cached_input_tokens":0,"output_tokens":1.5}}"#),
        Err(CodexEventError::InvalidUsage)
    );
    assert_eq!(
        parse_codex_events(&vec![b' '; 128 * 1024 + 1]),
        Err(CodexEventError::StreamTooLarge)
    );
}
