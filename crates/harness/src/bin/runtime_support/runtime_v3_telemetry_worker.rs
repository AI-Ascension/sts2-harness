fn worker_loop(
    state: Arc<ExporterState>,
    normal_rx: Receiver<TelemetryEvent>,
    critical_rx: Receiver<TelemetryEvent>,
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
    normal_rx: &Receiver<TelemetryEvent>,
    critical_rx: &Receiver<TelemetryEvent>,
) -> Option<TelemetryEvent> {
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
    normal_rx: &Receiver<TelemetryEvent>,
    critical_rx: &Receiver<TelemetryEvent>,
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

fn send_batch(state: &ExporterState, events: &[TelemetryEvent]) -> bool {
    let mut spans = Vec::with_capacity(events.len());
    for event in events {
        let sequence = state.sequence.fetch_add(1, Ordering::Relaxed);
        spans.push(render_span(&state.context, event, sequence));
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
    let mut stream = match TcpStream::connect_timeout(&ENDPOINT, SOCKET_TIMEOUT) {
        Ok(stream) => stream,
        Err(_) => return false,
    };
    if stream.set_read_timeout(Some(SOCKET_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(SOCKET_TIMEOUT)).is_err()
    {
        return false;
    }
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: 127.0.0.1:14318\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        OTLP_PATH,
        body.len()
    );
    if stream.write_all(request.as_bytes()).is_err() || stream.write_all(body).is_err() {
        let _ = stream.shutdown(Shutdown::Both);
        return false;
    }
    let mut response = vec![0_u8; MAX_RESPONSE_BYTES];
    let size = match stream.read(&mut response) {
        Ok(size) => size,
        Err(_) => {
            let _ = stream.shutdown(Shutdown::Both);
            return false;
        }
    };
    let mut total = size;
    while total < MAX_RESPONSE_BYTES {
        match stream.read(&mut response[total..]) {
            Ok(0) => break,
            Ok(read) => total += read,
            Err(_) => break,
        }
    }
    let response = &response[..total];
    let first_line = response
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    let accepted = first_line.starts_with(b"HTTP/1.1 2") || first_line.starts_with(b"HTTP/1.0 2");
    let partial_rejection = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .and_then(|header_end| serde_json::from_slice::<Value>(&response[header_end + 4..]).ok())
        .is_some_and(|value| contains_rejected_spans(&value));
    let _ = stream.shutdown(Shutdown::Both);
    accepted && !partial_rejection
}

fn contains_rejected_spans(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            (key == "rejectedSpans" && value.as_u64().is_some_and(|count| count > 0))
                || contains_rejected_spans(value)
        }),
        Value::Array(values) => values.iter().any(contains_rejected_spans),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}
