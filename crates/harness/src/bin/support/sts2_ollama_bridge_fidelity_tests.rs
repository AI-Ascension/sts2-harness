// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn prepared_ollama_input_retains_exact_serialized_body_only_when_opted_in() {
    let body = br#"{"model":"synthetic","messages":[{"role":"system","content":"fixture"}]}"#;
    let mut capture = sts2_harness::MemoryCapture::new(sts2_harness::CaptureMode::Memory, 4, LIMIT)
        .expect("capture config");
    PreparedOllamaInput::new(body).capture(&mut capture, "execution-8", None);
    let record = capture.records().next().expect("captured body");
    assert_eq!(record.component_kind.unwrap().as_str(), "opaque");
    assert_eq!(record.content.as_deref(), Some(body.as_slice()));

    let mut metadata =
        sts2_harness::MemoryCapture::new(sts2_harness::CaptureMode::Metadata, 4, LIMIT)
            .expect("capture config");
    PreparedOllamaInput::new(body).capture(&mut metadata, "execution-8", None);
    let record = metadata.records().next().expect("metadata record");
    assert!(record.content.is_none());
    assert!(record.sha256.is_none());
}

#[test]
fn actual_loopback_server_receives_the_same_serialized_body_as_capture()
-> Result<(), Box<dyn std::error::Error>> {
    use std::net::TcpListener;
    use std::thread;

    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let address = listener.local_addr()?;
    let server = thread::spawn(move || -> Result<Vec<u8>, String> {
        let (mut stream, _) = listener.accept().map_err(|_| "accept failed")?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|_| "timeout failed")?;
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let count = stream.read(&mut buffer).map_err(|_| "read failed")?;
            if count == 0 {
                return Err("request ended before body".to_owned());
            }
            request.extend_from_slice(&buffer[..count]);
            let Some(split) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let body_start = split + 4;
            let header_text = std::str::from_utf8(&request[..split]).map_err(|_| "headers utf8")?;
            let content_length = header_text
                .lines()
                .find_map(|line| line.strip_prefix("Content-Length: "))
                .ok_or("missing content length")?
                .parse::<usize>()
                .map_err(|_| "invalid content length")?;
            while request.len() < body_start + content_length {
                let count = stream.read(&mut buffer).map_err(|_| "body read failed")?;
                if count == 0 {
                    return Err("body ended early".to_owned());
                }
                request.extend_from_slice(&buffer[..count]);
            }
            let body = request[body_start..body_start + content_length].to_vec();
            let payload =
                br#"{"message":{"content":"{\"action_id\":\"combat.end-turn\",\"rationale\":\"synthetic oracle\"}"}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            )
            .map_err(|_| "response headers failed")?;
            stream
                .write_all(payload)
                .map_err(|_| "response body failed")?;
            return Ok(body);
        }
    });

    let request = json!({
        "model_execution_id":"oracle-execution-2",
        "legal_action_ids":["combat.end-turn"],
        "observation":{"state_id":"oracle-combat","generation":0},
    });
    let request_bytes = serde_json::to_vec(&request)?;
    let mut capture =
        sts2_harness::MemoryCapture::new(sts2_harness::CaptureMode::Memory, 4, LIMIT)?;
    run_with_capture_bytes(&request_bytes, &mut capture, address)?;
    let body = server
        .join()
        .map_err(|_| "server panicked")?
        .map_err(|error| error.to_owned())?;
    let record = capture
        .records()
        .find(|record| record.component_kind == Some(sts2_harness::CaptureComponentKind::Opaque))
        .ok_or("captured body is absent")?;
    assert_eq!(record.boundary, sts2_harness::CaptureBoundary::HttpBody);
    assert!(record.attempt_id.is_some());
    assert_eq!(record.content.as_deref(), Some(body.as_slice()));
    assert_eq!(record.observed_bytes, body.len());
    let lifecycle = capture
        .records()
        .find(|record| record.state == sts2_harness::TransportState::WriteCompleted)
        .ok_or("write completion is absent")?;
    assert_eq!(lifecycle.boundary, sts2_harness::CaptureBoundary::HttpBody);
    assert_eq!(
        lifecycle.parent_snapshot_id.as_deref(),
        Some(record.snapshot_id.as_str())
    );
    assert_ne!(lifecycle.snapshot_id, record.snapshot_id);
    Ok(())
}
