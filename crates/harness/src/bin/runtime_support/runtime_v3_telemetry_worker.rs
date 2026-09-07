// SPDX-License-Identifier: MIT

fn worker_loop(
    state: Arc<ExporterState>,
    normal_rx: Receiver<QueuedTelemetryEvent>,
    critical_rx: Receiver<QueuedTelemetryEvent>,
    control_rx: Receiver<ControlMessage>,
) -> WorkerReport {
    let mut report = WorkerReport::default();
    loop {
        if let Ok(control) = control_rx.try_recv() {
            match control {
                ControlMessage::Flush(reply) => {
                    drain_events(&state, &normal_rx, &critical_rx, &mut report);
                    let _ = reply.send(report.clone());
                    break;
                }
            }
        }
        let event = receive_event(&normal_rx, &critical_rx);
        if let Some(event) = event {
            let mut events = vec![event];
            while events.len() < MAX_BATCH {
                match critical_rx.try_recv().or_else(|_| normal_rx.try_recv()) {
                    Ok(next) => events.push(next),
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                }
            }
            if send_batch(&state, &events) {
                report.sent = report.sent.saturating_add(events.len() as u64);
            } else {
                report.failed = report.failed.saturating_add(events.len() as u64);
            }
        }
    }
    report
}

fn receive_event(
    normal_rx: &Receiver<QueuedTelemetryEvent>,
    critical_rx: &Receiver<QueuedTelemetryEvent>,
) -> Option<QueuedTelemetryEvent> {
    match critical_rx.try_recv() {
        Ok(event) => Some(event),
        Err(TryRecvError::Disconnected) => normal_rx.try_recv().ok(),
        Err(TryRecvError::Empty) => match normal_rx.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Disconnected) => {
                critical_rx.recv_timeout(Duration::from_millis(25)).ok()
            }
            Err(TryRecvError::Empty) => match critical_rx.recv_timeout(Duration::from_millis(25)) {
                Ok(event) => Some(event),
                Err(RecvTimeoutError::Timeout) => {
                    normal_rx.recv_timeout(Duration::from_millis(25)).ok()
                }
                Err(RecvTimeoutError::Disconnected) => {
                    normal_rx.recv_timeout(Duration::from_millis(25)).ok()
                }
            },
        },
    }
}

fn drain_events(
    state: &ExporterState,
    normal_rx: &Receiver<QueuedTelemetryEvent>,
    critical_rx: &Receiver<QueuedTelemetryEvent>,
    report: &mut WorkerReport,
) {
    loop {
        let mut events = Vec::new();
        while events.len() < MAX_BATCH {
            match critical_rx.try_recv().or_else(|_| normal_rx.try_recv()) {
                Ok(event) => events.push(event),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        if events.is_empty() {
            return;
        }
        if send_batch(state, &events) {
            report.sent = report.sent.saturating_add(events.len() as u64);
        } else {
            report.failed = report.failed.saturating_add(events.len() as u64);
        }
    }
}

fn send_batch(state: &ExporterState, events: &[QueuedTelemetryEvent]) -> bool {
    let mut spans = Vec::with_capacity(events.len());
    for queued in events {
        spans.push(render_span_at(
            &state.context,
            &queued.event,
            queued.sequence,
            queued.enqueued_at_unix_nanos,
        ));
    }
    let body = json!({
        "resourceSpans": [{
            "resource": {"attributes": [
                {"key": "service.name", "value": {"stringValue": "sts2-harness"}},
                {"key": "service.version", "value": {"stringValue": "runtime-v3"}},
                {"key": "deployment.environment", "value": {"stringValue": "local"}}
            ]},
            "scopeSpans": [{
                "scope": {"name": "sts2.runtime.telemetry", "version": "1"},
                "spans": spans
            }]
        }]
    });
    let bytes = match serde_json::to_vec(&body) {
        Ok(bytes) if bytes.len() <= MAX_BODY_BYTES => bytes,
        _ => return false,
    };
    post_otlp(&bytes)
}

fn post_otlp(body: &[u8]) -> bool {
    post_otlp_to(&ENDPOINT, body)
}

fn post_otlp_to(endpoint: &SocketAddr, body: &[u8]) -> bool {
    let mut stream = match TcpStream::connect_timeout(endpoint, SOCKET_TIMEOUT) {
        Ok(stream) => stream,
        Err(_) => return false,
    };
    let deadline = Instant::now() + SOCKET_TIMEOUT;
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {endpoint}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        OTLP_PATH,
        body.len()
    );
    if !write_with_deadline(&mut stream, request.as_bytes(), deadline)
        || !write_with_deadline(&mut stream, body, deadline)
    {
        let _ = stream.shutdown(Shutdown::Both);
        return false;
    }
    let accepted = read_otlp_response(&mut stream, deadline)
        .is_some_and(|response| response.status / 100 == 2 && valid_otlp_success_body(&response.body));
    let _ = stream.shutdown(Shutdown::Both);
    accepted
}

struct OtlpHttpResponse {
    status: u16,
    body: Vec<u8>,
}

fn write_with_deadline(stream: &mut TcpStream, mut bytes: &[u8], deadline: Instant) -> bool {
    while !bytes.is_empty() {
        let Some(timeout) = deadline.checked_duration_since(Instant::now()) else {
            return false;
        };
        if stream.set_write_timeout(Some(timeout)).is_err() {
            return false;
        }
        let Ok(written) = stream.write(bytes) else {
            return false;
        };
        if written == 0 {
            return false;
        }
        bytes = &bytes[written..];
    }
    deadline.checked_duration_since(Instant::now()).is_some()
}

fn read_with_deadline(stream: &mut TcpStream, bytes: &mut [u8], deadline: Instant) -> Option<usize> {
    let timeout = deadline.checked_duration_since(Instant::now())?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.read(bytes).ok()
}

fn read_otlp_response(stream: &mut TcpStream, deadline: Instant) -> Option<OtlpHttpResponse> {
    const MAX_HEADER_BYTES: usize = 8 * 1024;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 2048];
    let header_end = loop {
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            if end + 4 > MAX_HEADER_BYTES {
                return None;
            }
            break end;
        }
        if bytes.len() >= MAX_HEADER_BYTES {
            return None;
        }
        let read = read_with_deadline(stream, &mut buffer, deadline)?;
        if read == 0 {
            return None;
        }
        bytes.extend_from_slice(&buffer[..read]);
    };
    let header = std::str::from_utf8(&bytes[..header_end]).ok()?;
    let mut lines = header.split("\r\n");
    let status_line = lines.next()?;
    let mut status_parts = status_line.split_ascii_whitespace();
    if !matches!(status_parts.next(), Some("HTTP/1.1" | "HTTP/1.0")) {
        return None;
    }
    let code = status_parts.next()?;
    if code.len() != 3 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let status = code.parse::<u16>().ok()?;
    if !(100..600).contains(&status) || !valid_http_value(status_line) {
        return None;
    }

    let mut content_length = None;
    let mut header_names = std::collections::BTreeSet::new();
    for line in lines {
        let (name, value) = line.split_once(':')?;
        if !valid_http_name(name)
            || !valid_http_value(value)
            || !header_names.insert(name.to_ascii_lowercase())
        {
            return None;
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return None;
        }
        if name.eq_ignore_ascii_case("content-length") {
            let value = value.trim_matches(' ');
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            let length = value.parse::<usize>().ok()?;
            if length > MAX_RESPONSE_BYTES || content_length.replace(length).is_some() {
                return None;
            }
        }
    }
    let content_length = content_length?;
    let body_start = header_end + 4;
    let available = bytes.len().saturating_sub(body_start);
    if available > content_length {
        return None;
    }
    let mut body = bytes[body_start..].to_vec();
    while body.len() < content_length {
        let remaining = content_length - body.len();
        let read_capacity = remaining.min(buffer.len());
        let read = read_with_deadline(stream, &mut buffer[..read_capacity], deadline)?;
        if read == 0 {
            return None;
        }
        body.extend_from_slice(&buffer[..read]);
    }
    Some(OtlpHttpResponse { status, body })
}

fn valid_http_name(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
        })
}

fn valid_http_value(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte == b' ' || byte.is_ascii_graphic())
}

fn valid_otlp_success_body(body: &[u8]) -> bool {
    let Ok(Value::Object(response)) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    response.iter().all(|(key, value)| match key.as_str() {
        "partialSuccess" => valid_partial_success(value),
        _ => false,
    })
}

fn valid_partial_success(value: &Value) -> bool {
    let Value::Object(partial) = value else {
        return false;
    };
    partial.iter().all(|(key, value)| match key.as_str() {
        "rejectedSpans" => rejected_spans_count(value).is_some_and(|count| count == 0),
        "errorMessage" => value.is_string(),
        _ => false,
    })
}

fn rejected_spans_count(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(value) => value.parse::<u64>().ok(),
        _ => None,
    }
}
