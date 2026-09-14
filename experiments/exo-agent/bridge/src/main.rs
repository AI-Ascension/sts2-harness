// SPDX-License-Identifier: MIT

mod turn;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

const INPUT_LIMIT: u64 = 160 * 1024;
const OUTPUT_LIMIT: usize = 16 * 1024;

/// Private internal handoff from sts2-exo-bridge, never a model-visible control envelope.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invocation {
    version: String,
    request_id: String,
    host_turn_id: String,
    model: String,
    endpoint: String,
    module_path: PathBuf,
    source_root: PathBuf,
    state_root: PathBuf,
    input: Value,
    timeout_millis: u32,
    max_output_tokens: u32,
    credential: String,
}

#[derive(Serialize)]
struct Receipt {
    version: &'static str,
    request_id: String,
    host_turn_id: String,
    exo_turn_id: String,
    exo_session_id: String,
    decision: Option<String>,
    error_code: Option<&'static str>,
    fetch_attempts: u64,
    forwarded_requests: u64,
    denied_requests: u64,
}

#[tokio::main]
async fn main() {
    if let Err(code) = run().await {
        // Upstream errors can contain paths, credentials and model output. Never forward them.
        eprintln!("{code}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), &'static str> {
    let mut bytes = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::io::stdin()
            .take(INPUT_LIMIT + 1)
            .read_to_end(&mut bytes),
    )
    .await
    .map_err(|_| "exo_executor_input_timeout")?
    .map_err(|_| "exo_executor_input")?;
    if bytes.len() > INPUT_LIMIT as usize {
        return Err("exo_executor_input_bound");
    }
    let invocation: Invocation =
        serde_json::from_slice(&bytes).map_err(|_| "exo_executor_input_shape")?;
    if invocation.version != "sts2.exo-executor-input-v1"
        || invocation.timeout_millis == 0
        || invocation.timeout_millis > 120_000
        || invocation.max_output_tokens == 0
        || invocation.max_output_tokens > 4096
        || !invocation.input.is_object()
    {
        return Err("exo_executor_input_invalid");
    }
    let duration = std::time::Duration::from_millis(u64::from(invocation.timeout_millis));
    let receipt = tokio::time::timeout(duration, turn::execute(invocation))
        .await
        .map_err(|_| "exo_executor_turn_timeout")??;
    let output = serde_json::to_vec(&receipt).map_err(|_| "exo_executor_receipt")?;
    if output.len() > OUTPUT_LIMIT {
        return Err("exo_executor_receipt_bound");
    }
    let mut stdout = tokio::io::stdout();
    publish(&mut stdout, &output).await
}

async fn publish(writer: &mut (impl AsyncWrite + Unpin), bytes: &[u8]) -> Result<(), &'static str> {
    writer
        .write_all(bytes)
        .await
        .map_err(|_| "exo_executor_output")?;
    // Tokio can accept bytes before its blocking stdout write completes. Keep and flush this handle.
    writer.flush().await.map_err(|_| "exo_executor_output")
}

#[cfg(test)]
mod output_tests;
