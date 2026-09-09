// SPDX-License-Identifier: MIT

const MAX_REPLAY_EVENT_BYTES: usize = 512 * 1024;

#[cfg(test)]
use std::cell::RefCell;

#[cfg(test)]
thread_local! {
    static REPLAY_CAPTURE: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
}

fn replay_enabled() -> bool {
    std::env::var("STS2_LIVE_EPISODE").as_deref() == Ok("true")
}

fn encode_replay_event(event: &serde_json::Value) -> Option<Vec<u8>> {
    let mut bytes = serde_json::to_vec(event).ok()?;
    if bytes.len().checked_add(1)? > MAX_REPLAY_EVENT_BYTES {
        return None;
    }
    bytes.push(b'\n');
    Some(bytes)
}

fn emit_replay_event(event: serde_json::Value) {
    let bytes = encode_replay_event(&event).or_else(|| {
        // A bounded event must never disappear silently: the marker is intentionally a
        // parser-failing record so a truncated stream cannot masquerade as a complete replay.
        let marker = json!({
            "event": "replay_stream_truncated",
            "source_event": replay_event_name(&event),
            "reason": "event_exceeds_bound"
        });
        encode_replay_event(&marker)
    });
    let Some(bytes) = bytes else {
        return;
    };
    emit_replay_bytes(&bytes);
}

fn replay_event_name(event: &serde_json::Value) -> &'static str {
    match event["event"].as_str() {
        Some("model_decision") => "model_decision",
        Some("action_receipt") => "action_receipt",
        Some("operation_wait_completed") => "operation_wait_completed",
        Some("episode_complete") => "episode_complete",
        Some("episode_failed") => "episode_failed",
        _ => "unknown",
    }
}

fn emit_replay_bytes(bytes: &[u8]) {
    #[cfg(test)]
    if REPLAY_CAPTURE.with(|capture| {
        let mut capture = capture.borrow_mut();
        if let Some(output) = capture.as_mut() {
            output.extend_from_slice(bytes);
            true
        } else {
            false
        }
    }) {
        return;
    }
    if !replay_enabled() {
        return;
    }
    let mut stdout = std::io::stdout().lock();
    let _ = std::io::Write::write_all(&mut stdout, bytes);
}

/// Captures the same bounded event bytes used by the stdout sink for deterministic integration
/// tests. The hook is test-only and thread-local so it cannot alter a live worker's stream.
#[cfg(test)]
pub(super) fn capture_replay_events<F, T>(operation: F) -> (T, Vec<u8>)
where
    F: FnOnce() -> T,
{
    REPLAY_CAPTURE.with(|capture| {
        assert!(
            capture.borrow().is_none(),
            "replay capture was already active"
        );
        *capture.borrow_mut() = Some(Vec::new());
    });
    let result = operation();
    let bytes = REPLAY_CAPTURE.with(|capture| capture.borrow_mut().take().unwrap_or_default());
    (result, bytes)
}

/// Flushes the private replay stream without changing the episode's result when the stream
/// itself cannot be flushed. The caller deliberately keeps the original failure authoritative.
pub(super) fn flush_replay_stream() -> std::io::Result<()> {
    if replay_enabled() {
        std::io::Write::flush(&mut std::io::stdout().lock())
    } else {
        Ok(())
    }
}

