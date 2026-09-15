// SPDX-License-Identifier: MIT
//! Maximum source through the real pinned native tool/result accumulation path.
use super::*;
use std::path::Path;

pub fn run(
    root: &Path,
    binary: &Path,
    config: &Path,
    temporary: &Path,
    model: &Model,
) -> Result<usize> {
    let source = (0_u8..=255).cycle().take(65_536).collect::<Vec<_>>();
    model.set(200, tool_response("maximum", false))?;
    let mut child = Command::new(binary)
        .arg("--lookup-synthetic")
        .arg(config)
        .arg(digest(config)?)
        .env_clear()
        .env("TMPDIR", temporary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().ok_or("stdin")?;
    let mut output = BufReader::new(child.stdout.take().ok_or("stdout")?);
    let request: Value = serde_json::from_slice(&std::fs::read(
        root.join("protocol-artifact/exo-bridge-v1/golden/request.json"),
    )?)?;
    let action = request["legal_action_ids"][0]
        .as_str()
        .ok_or("action")?
        .to_owned();
    let mut frame = json!({"wire_version":"sts2.exo-lookup-wire-v1","request_id":"maximum-private-request",
        "turn_id":"maximum-private-turn","sequence":0,"payload":{"kind":"start","request":request,
            "optional_byte_budget":7000}});
    write_frame(&mut input, &frame)?;
    let first = read(&mut output)?;
    assert_eq!(first["sequence"], 1);
    assert_eq!(first["payload"]["kind"], "query");
    model.replace_response(200, read_response(0))?;
    frame["sequence"] = json!(1);
    frame["payload"] = json!({"kind":"feedback","value":{"record_ordinal":0,"data":{
        "authority":"untrusted_game_information_data","delivery":"retained","byte_length":65536}}});
    write_frame(&mut input, &frame)?;
    for (index, chunk) in source.chunks(3000).enumerate() {
        let offset = index * 3000;
        let tool = read(&mut output)?;
        assert_eq!(tool["sequence"], index + 2);
        assert_eq!(
            tool["payload"],
            json!({"kind":"read_retained","record_ordinal":0,"offset":offset})
        );
        let next = offset + chunk.len();
        let reply = if next < source.len() {
            read_response(next)
        } else {
            response(
                &serde_json::to_string(&json!({"action_id":action}))?,
                "message",
            )
        };
        model.replace_response(200, reply)?;
        frame["sequence"] = json!(index + 2);
        frame["payload"] = json!({"kind":"feedback","value":{"record_ordinal":0,"offset":offset,
            "next_offset":next,"total_bytes":65536,"encoding":"hex","bytes":hex(chunk),
            "authority":"untrusted_game_information_data"}});
        assert!(serde_json::to_vec(&frame["payload"]["value"])?.len() <= 7000);
        write_frame(&mut input, &frame)?;
    }
    let decision = read(&mut output)?;
    assert_eq!(decision["sequence"], 24);
    assert_eq!(
        decision["payload"],
        json!({"kind":"decision","action_id":action})
    );
    drop(input);
    let result = child.wait_with_output()?;
    assert!(
        result.status.success(),
        "maximum: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(model.request_count(), 24);
    let requests = model.requests.lock().map_err(|_| "poisoned")?;
    let maximum = requests
        .iter()
        .map(serde_json::to_vec)
        .collect::<std::result::Result<Vec<_>, _>>()?
        .iter()
        .map(Vec::len)
        .max()
        .ok_or("requests missing")?;
    assert!(maximum <= 160 * 1024);
    let last = requests.last().ok_or("last request missing")?;
    let outputs = last["input"]
        .as_array()
        .ok_or("model inputs")?
        .iter()
        .filter(|item| item["type"] == "function_call_output");
    let mut reconstructed = Vec::new();
    let mut count = 0;
    for item in outputs {
        let text = item["output"].as_str().ok_or("tool result text")?;
        assert!(text.len() <= 7000);
        let value: Value = serde_json::from_str(text)?;
        assert!(value.get("preview").is_none() && value.get("resultArtifact").is_none());
        if value["encoding"] != "hex" {
            continue;
        }
        assert_eq!(value["offset"], reconstructed.len());
        let encoded = value["bytes"].as_str().ok_or("hex result")?.as_bytes();
        for pair in encoded.chunks_exact(2) {
            reconstructed.push(u8::from_str_radix(std::str::from_utf8(pair)?, 16)?);
        }
        count += 1;
    }
    assert_eq!(count, 22);
    assert_eq!(reconstructed, source);
    Ok(maximum)
}

fn read(output: &mut impl BufRead) -> Result<Value> {
    let mut line = String::new();
    output.read_line(&mut line)?;
    serde_json::from_str(&line).map_err(Into::into)
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn read_response(offset: usize) -> Value {
    let mut value = response("", "tool");
    value["output"][0] = json!({"type":"function_call","id":format!("fc_read_{offset}"),
        "call_id":format!("call_read_{offset}"),"name":"sts2_lookup_read",
        "arguments":json!({"record_ordinal":0,"offset":offset}).to_string(),"status":"completed"});
    value
}
