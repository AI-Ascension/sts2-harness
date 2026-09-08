// SPDX-License-Identifier: MIT

fn supervised_call<T: Send>(
    work: impl FnOnce() -> Result<T, McpProcessError> + Send,
) -> Result<T, McpProcessError> {
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name(String::from("mcp-exchange"))
            .spawn_scoped(scope, work)
            .map_err(|_| {
                McpProcessError::new(
                    McpProcessErrorKind::SupervisorUnavailable,
                    "MCP supervisor unavailable",
                )
            })?;
        worker.join().map_err(|_| {
            McpProcessError::new(
                McpProcessErrorKind::SupervisorFailed,
                "MCP supervisor failed",
            )
        })?
    })
}

fn supervised<T: Send>(work: impl FnOnce() -> Result<T, &'static str> + Send) -> Result<T, String> {
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name(String::from("mcp-exchange"))
            .spawn_scoped(scope, work)
            .map_err(|_| String::from("MCP supervisor unavailable"))?;
        worker
            .join()
            .map_err(|_| String::from("MCP supervisor failed"))?
            .map_err(String::from)
    })
}

async fn read_frame(output: &mut BufReader<ChildStdout>) -> Result<Vec<u8>, McpProcessError> {
    let mut bytes = Vec::new();
    loop {
        let available = output.fill_buf().await.map_err(|_| {
            McpProcessError::new(
                McpProcessErrorKind::ResponseRead,
                "MCP response read failed",
            )
        })?;
        if available.is_empty() {
            return Err(McpProcessError::new(
                McpProcessErrorKind::ResponseEof,
                "MCP response ended before its delimiter",
            ));
        }
        let delimiter = available.iter().position(|byte| *byte == b'\n');
        let count = delimiter.map_or(available.len(), |index| index + 1);
        if bytes.len().saturating_add(count) > MAX_RESPONSE_BYTES {
            return Err(McpProcessError::new(
                McpProcessErrorKind::Protocol,
                "MCP response exceeded its size limit",
            ));
        }
        bytes.extend_from_slice(&available[..count]);
        output.consume(count);
        if delimiter.is_some() {
            return Ok(bytes);
        }
    }
}

fn validate_response(bytes: &[u8], id: u64) -> Result<Value, McpProcessError> {
    let response: Value = serde_json::from_slice(bytes).map_err(|_| {
        McpProcessError::new(McpProcessErrorKind::Protocol, "MCP response was not JSON")
    })?;
    if !response.is_object()
        || response["jsonrpc"] != "2.0"
        || response["id"].as_u64() != Some(id)
        || response.get("result").is_some() == response.get("error").is_some()
    {
        return Err(McpProcessError::new(
            McpProcessErrorKind::Protocol,
            "MCP response envelope was invalid",
        ));
    }
    Ok(response)
}
