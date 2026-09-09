// SPDX-License-Identifier: MIT

fn worker_loop(
    state: Arc<ExporterState>,
    telemetry_rx: Receiver<QueuedTelemetryEvent>,
    control_rx: Receiver<ControlMessage>,
) -> WorkerReport {
    let mut report = WorkerReport::default();
    loop {
        if let Ok(control) = control_rx.try_recv() {
            match control {
                ControlMessage::Flush(reply) => {
                    drain_events(&state, &telemetry_rx, &mut report);
                    let dropped_events = state
                        .normal_dropped
                        .load(Ordering::Relaxed)
                        .saturating_add(state.critical_dropped.load(Ordering::Relaxed));
                    let status = report.export_status(dropped_events);
                    let status_event = QueuedTelemetryEvent {
                        event: TelemetryEvent::ExportStatus {
                            status,
                            sent: report.sent,
                            failed: report.failed,
                            dropped_events,
                        },
                        enqueued_at_unix_nanos: unix_nanos(),
                        sequence: state.sequence.fetch_add(1, Ordering::Relaxed),
                    };
                    if send_batch(&state, std::slice::from_ref(&status_event)) {
                        report.sent = report.sent.saturating_add(1);
                    } else {
                        report.failed = report.failed.saturating_add(1);
                    }
                    let _ = reply.send(report.clone());
                    break;
                }
            }
        }
        match telemetry_rx.recv_timeout(Duration::from_millis(25)) {
            Ok(event) => {
            let mut events = vec![event];
            while events.len() < MAX_BATCH {
                match telemetry_rx.try_recv() {
                    Ok(next) => events.push(next),
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                }
            }
            send_ordered_events(&state, &events, &mut report);
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    report
}

fn drain_events(
    state: &ExporterState,
    telemetry_rx: &Receiver<QueuedTelemetryEvent>,
    report: &mut WorkerReport,
) {
    let mut events = Vec::new();
    while let Ok(event) = telemetry_rx.try_recv() {
        events.push(event);
    }
    send_ordered_events(state, &events, report);
}

fn send_ordered_events(
    state: &ExporterState,
    events: &[QueuedTelemetryEvent],
    report: &mut WorkerReport,
) {
    let mut segment_start = 0;
    for (index, event) in events.iter().enumerate() {
        if matches!(event.event, TelemetryEvent::RunFinished { .. }) {
            send_segments(state, &events[segment_start..index], report);
            send_segments(state, &events[index..=index], report);
            segment_start = index + 1;
        }
    }
    send_segments(state, &events[segment_start..], report);
}

fn send_segments(
    state: &ExporterState,
    events: &[QueuedTelemetryEvent],
    report: &mut WorkerReport,
) {
    for batch in events.chunks(MAX_BATCH) {
        if send_batch(state, batch) {
            report.sent = report.sent.saturating_add(batch.len() as u64);
        } else {
            report.failed = report.failed.saturating_add(batch.len() as u64);
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

include!("runtime_v3_telemetry_worker_http.rs");
