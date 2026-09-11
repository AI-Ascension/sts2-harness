// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use sts2_harness::{CodexEventAccounting, CodexStreamStatus, CodexTokenUsage, CodexUsageStatus};

const MAX_ACCOUNTING_BYTES: usize = 16 * 1024;
const ACCOUNTING_PATH_VARIABLE: &str = "STS2_PROVIDER_ACCOUNTING_PATH";
const ACCOUNTING_SCHEMA: &str = "sts2.provider-accounting-v1";
const ACCOUNTING_SOURCE: &str = "codex.exec.jsonl";

pub(super) struct CapturedStream {
    pub(super) bytes: Vec<u8>,
    pub(super) total_bytes: usize,
    pub(super) truncated: bool,
    pub(super) read_error: bool,
}

pub(super) struct ProviderExecution {
    pub(super) completed: bool,
    pub(super) input_written: bool,
    pub(super) stdout_invalid: bool,
    pub(super) stderr_invalid: bool,
    pub(super) stdout_bytes: usize,
    pub(super) stderr_bytes: usize,
}

pub(super) fn read_decision(path: &Path) -> (Result<String, std::io::Error>, Option<String>) {
    let mut bytes = Vec::new();
    let result = File::open(path).and_then(|file| file.take(8193).read_to_end(&mut bytes));
    if result.is_err() {
        return (
            result.and_then(|_| String::from_utf8(bytes).map_err(std::io::Error::other)),
            None,
        );
    }
    let digest = digest(&bytes);
    let content = String::from_utf8(bytes).map_err(std::io::Error::other);
    (content, Some(digest))
}

pub(super) fn capture_stream<R: Read + Send + 'static>(
    mut stream: R,
    maximum: usize,
) -> JoinHandle<CapturedStream> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        let mut total_bytes = 0_usize;
        let mut truncated = false;
        let mut read_error = false;
        loop {
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    total_bytes = total_bytes.saturating_add(count);
                    let remaining = maximum.saturating_sub(bytes.len());
                    let retained = remaining.min(count);
                    bytes.extend_from_slice(&buffer[..retained]);
                    truncated |= retained != count;
                }
                Err(_) => {
                    read_error = true;
                    break;
                }
            }
        }
        CapturedStream {
            bytes,
            total_bytes,
            truncated,
            read_error,
        }
    })
}

pub(super) fn accounting_record(
    request: &Value,
    request_bytes: &[u8],
    events: &CodexEventAccounting,
    execution: &ProviderExecution,
    decision_digest: Option<&str>,
    decision_valid: bool,
) -> Value {
    let usage = events.usage.as_ref().map(token_usage);
    json!({
        "schema": ACCOUNTING_SCHEMA,
        "source": ACCOUNTING_SOURCE,
        "provider": "openai",
        "model": "gpt-6-astra",
        "provider_request_id": events.provider_request_id,
        "provider_request_id_kind": "codex_thread_id",
        "provider_request_identity_status": if events.provider_request_id.is_some() { "reported" } else { "unavailable" },
        "harness_model_execution_id": request["model_execution_id"].as_str(),
        "request_sha256": digest(request_bytes),
        "decision_sha256": decision_digest,
        "execution_status": if execution.completed { "completed" } else { "failed" },
        "decision_status": if decision_valid { "valid" } else { "invalid" },
        "input_written": execution.input_written,
        "event_stream_status": if execution.stdout_invalid { "invalid" } else { events.stream_status.as_str() },
        "stderr_stream_status": if execution.stderr_invalid { "invalid" } else { "discarded" },
        "stdout_bytes": execution.stdout_bytes,
        "stderr_bytes": execution.stderr_bytes,
        "event_count": events.event_count,
        "turn_count": events.turn_count,
        "completed_turn_count": events.completed_turn_count,
        "usage_status": events.usage_status.as_str(),
        "usage": usage,
    })
}

pub(super) fn invalid_event_accounting() -> CodexEventAccounting {
    CodexEventAccounting {
        provider_request_id: None,
        event_count: 0,
        turn_count: 0,
        completed_turn_count: 0,
        usage_status: CodexUsageStatus::Unavailable,
        usage: None,
        stream_status: CodexStreamStatus::Invalid,
    }
}

fn token_usage(usage: &CodexTokenUsage) -> Value {
    json!({
        "input_tokens": usage.input_tokens,
        "cached_input_tokens": usage.cached_input_tokens,
        "cache_write_input_tokens": usage.cache_write_input_tokens,
        "output_tokens": usage.output_tokens,
        "reasoning_output_tokens": usage.reasoning_output_tokens,
    })
}

pub(super) fn write_accounting(record: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let Some(path) = accounting_path()? else {
        return Ok(());
    };
    let bytes = serde_json::to_vec(record)?;
    if bytes.len() > MAX_ACCOUNTING_BYTES {
        return Err("provider accounting record exceeds its bound".into());
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true).write(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if file.metadata()?.permissions().mode() & 0o077 != 0 {
            return Err("provider accounting file is not owner-only".into());
        }
    }
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_data()?;
    Ok(())
}

fn accounting_path() -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
    let Some(value) = std::env::var_os(ACCOUNTING_PATH_VARIABLE) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .ok_or("provider accounting path is not UTF-8")?;
    let path = PathBuf::from(value);
    if value.is_empty()
        || value.len() > 4096
        || !path.is_absolute()
        || value.chars().any(char::is_control)
    {
        return Err("provider accounting path is invalid".into());
    }
    Ok(Some(path))
}

pub(super) fn digest(bytes: &[u8]) -> String {
    sts2_harness::sha256_hex(bytes)
}
