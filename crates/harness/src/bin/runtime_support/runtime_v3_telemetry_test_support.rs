// SPDX-License-Identifier: MIT

#[cfg(test)]
impl TelemetryHandle {
    fn with_test_sink(context: TelemetryContext) -> (Self, Receiver<QueuedTelemetryEvent>) {
        let (telemetry_tx, telemetry_rx) = mpsc::sync_channel(NORMAL_QUEUE_CAPACITY);
        let state = ExporterState {
            context,
            telemetry_tx,
            admission: Mutex::new(()),
            closed: AtomicBool::new(false),
            sequence: AtomicU64::new(1),
            normal_dropped: AtomicU64::new(0),
            critical_dropped: AtomicU64::new(0),
        };
        (
            Self {
                state: Arc::new(state),
            },
            telemetry_rx,
        )
    }
}
