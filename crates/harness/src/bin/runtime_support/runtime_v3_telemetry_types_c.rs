// SPDX-License-Identifier: MIT

impl RuntimeV3Telemetry {
    pub fn new(context: TelemetryContext) -> Self {
        let (normal_tx, normal_rx) = mpsc::sync_channel(NORMAL_QUEUE_CAPACITY);
        let (critical_tx, critical_rx) = mpsc::sync_channel(CRITICAL_QUEUE_CAPACITY);
        let (control_tx, control_rx) = mpsc::sync_channel(1);
        let state = Arc::new(ExporterState {
            context,
            normal_tx,
            critical_tx,
            closed: AtomicBool::new(false),
            sequence: AtomicU64::new(1),
            normal_dropped: AtomicU64::new(0),
            critical_dropped: AtomicU64::new(0),
        });
        let worker_state = Arc::clone(&state);
        let worker = thread::Builder::new()
            .name(String::from("sts2-telemetry"))
            .spawn(move || worker_loop(worker_state, normal_rx, critical_rx, control_rx))
            .ok();
        Self {
            handle: TelemetryHandle { state },
            control_tx,
            worker,
        }
    }

    pub fn handle(&self) -> TelemetryHandle {
        self.handle.clone()
    }

    pub fn finish(mut self, deadline: Duration) -> FlushReport {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        let control_status = self.control_tx.try_send(ControlMessage::Flush(reply_tx));
        let mut report = match control_status {
            Ok(()) => match reply_rx.recv_timeout(deadline) {
                Ok(worker) => FlushReport {
                    sent: worker.sent,
                    failed: worker.failed,
                    normal_dropped: self.handle.state.normal_dropped.load(Ordering::Relaxed),
                    critical_dropped: self.handle.state.critical_dropped.load(Ordering::Relaxed),
                    timed_out: false,
                },
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => FlushReport {
                    normal_dropped: self.handle.state.normal_dropped.load(Ordering::Relaxed),
                    critical_dropped: self.handle.state.critical_dropped.load(Ordering::Relaxed),
                    timed_out: true,
                    ..FlushReport::default()
                },
            },
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => FlushReport {
                normal_dropped: self.handle.state.normal_dropped.load(Ordering::Relaxed),
                critical_dropped: self.handle.state.critical_dropped.load(Ordering::Relaxed),
                timed_out: true,
                ..FlushReport::default()
            },
        };
        self.handle.state.closed.store(true, Ordering::Release);
        let normal_dropped = self.handle.state.normal_dropped.load(Ordering::Relaxed);
        let critical_dropped = self.handle.state.critical_dropped.load(Ordering::Relaxed);
        drop(self.handle);
        drop(self.control_tx);
        if let Some(worker) = self.worker.take() {
            if report.timed_out {
                // `JoinHandle::join` has no deadline. The worker owns only a loopback
                // socket with bounded I/O timeouts, so detach it after a timed-out
                // flush rather than extending the gameplay shutdown indefinitely.
                drop(worker);
            } else if worker.join().is_err() {
                report.failed = report.failed.saturating_add(1);
            }
        }
        report.normal_dropped = report.normal_dropped.max(normal_dropped);
        report.critical_dropped = report.critical_dropped.max(critical_dropped);
        report
    }
}

impl TelemetryHandle {
    #[cfg(test)]
    pub fn disabled() -> Self {
        let (normal_tx, _normal_rx) = mpsc::sync_channel(0);
        let (critical_tx, _critical_rx) = mpsc::sync_channel(0);
        let state = ExporterState {
            context: TelemetryContext {
                run_id: String::from("disabled-run"),
                episode_id: String::from("disabled-episode"),
                trajectory_id: String::from("disabled-trajectory"),
                trace_id: String::from("disabled-trace"),
                instance_id: String::from("disabled-instance"),
                session_id: String::from("disabled-session"),
                runtime_profile: String::from("disabled"),
                schema_version: String::from("disabled"),
                provider_revision_digest: String::from("disabled"),
            },
            normal_tx,
            critical_tx,
            closed: AtomicBool::new(true),
            sequence: AtomicU64::new(1),
            normal_dropped: AtomicU64::new(0),
            critical_dropped: AtomicU64::new(0),
        };
        Self {
            state: Arc::new(state),
        }
    }

    pub fn run_started(&self) -> EnqueueStatus {
        self.enqueue(TelemetryEvent::RunStarted, true)
    }

    pub fn model_decision(
        &self,
        model_execution_id: u64,
        decision_kind: DecisionKind,
        action_id: Option<&str>,
        operation_id: Option<&str>,
        confidence: Option<u8>,
    ) -> EnqueueStatus {
        self.enqueue(
            TelemetryEvent::ModelDecision {
                model_execution_id,
                decision_kind,
                action_id_digest: action_id.map(|value| digest("action", value)),
                operation_id_digest: operation_id.map(|value| digest("operation", value)),
                confidence: confidence.filter(|value| *value <= 100),
            },
            false,
        )
    }

    pub fn model_failure(
        &self,
        model_execution_id: u64,
        failure_code: FailureCode,
    ) -> EnqueueStatus {
        self.enqueue(
            TelemetryEvent::ModelFailure {
                model_execution_id,
                failure_code,
            },
            false,
        )
    }

    pub fn observation(
        &self,
        source: ObservationSource,
        observation: &EpisodeObservation,
    ) -> EnqueueStatus {
        self.enqueue(
            TelemetryEvent::Observation {
                source,
                generation: observation.generation(),
                stage: observation.stage().into(),
                state_id_digest: digest("state", observation.state_id()),
            },
            false,
        )
    }

    pub fn action_dispatch(
        &self,
        operation_id: &str,
        action_id: &str,
        action_kind: ActionKind,
        generation: u64,
        status: DispatchStatus,
        failure_code: Option<FailureCode>,
    ) -> EnqueueStatus {
        self.enqueue(
            TelemetryEvent::ActionDispatch {
                operation_id_digest: digest("operation", operation_id),
                action_id_digest: digest("action", action_id),
                action_kind: action_kind.into(),
                generation,
                status: status.into(),
                failure_code,
            },
            false,
        )
    }

    pub fn settlement(
        &self,
        operation_id: &str,
        action_id: &str,
        from_generation: u64,
        after: &EpisodeObservation,
        effect_kind: &str,
        source: ObservationSource,
    ) -> EnqueueStatus {
        self.enqueue(
            TelemetryEvent::SettlementObservation {
                operation_id_digest: digest("operation", operation_id),
                action_id_digest: digest("action", action_id),
                from_generation,
                to_generation: after.generation(),
                stage: after.stage().into(),
                effect_class: effect_class(effect_kind),
                effect_digest: digest("effect", effect_kind),
                source,
            },
            false,
        )
    }

    pub fn recovery(
        &self,
        kind: RecoveryKind,
        operation_id: Option<&str>,
        attempt: u8,
        outcome: &'static str,
        failure_code: Option<FailureCode>,
    ) -> EnqueueStatus {
        self.enqueue(
            TelemetryEvent::Recovery {
                kind,
                operation_id_digest: operation_id.map(|value| digest("operation", value)),
                attempt,
                outcome,
                failure_code,
            },
            false,
        )
    }

    pub fn failure(
        &self,
        boundary: &'static str,
        failure_code: FailureCode,
        retryable: bool,
        operation_id: Option<&str>,
    ) -> EnqueueStatus {
        self.enqueue(
            TelemetryEvent::Failure {
                boundary,
                failure_code,
                retryable,
                operation_id_digest: operation_id.map(|value| digest("operation", value)),
            },
            false,
        )
    }

    pub fn terminal(
        &self,
        observation: &EpisodeObservation,
        outcome: GameOutcome,
    ) -> EnqueueStatus {
        self.enqueue(
            TelemetryEvent::TerminalObserved {
                stage: observation.stage().into(),
                outcome,
                generation: observation.generation(),
                state_id_digest: digest("state", observation.state_id()),
            },
            true,
        )
    }

    pub fn run_finished(
        &self,
        outcome: GameOutcome,
        terminal_stage: TelemetryStage,
        cleanup_status: CleanupStatus,
    ) -> EnqueueStatus {
        let dropped = self
            .state
            .normal_dropped
            .load(Ordering::Relaxed)
            .saturating_add(self.state.critical_dropped.load(Ordering::Relaxed));
        self.enqueue(
            TelemetryEvent::RunFinished {
                outcome,
                terminal_stage,
                cleanup_status,
                dropped_events: dropped,
            },
            true,
        )
    }

    fn enqueue(&self, event: TelemetryEvent, critical: bool) -> EnqueueStatus {
        if self.state.closed.load(Ordering::Acquire) {
            return EnqueueStatus::Closed;
        }
        let event = QueuedTelemetryEvent {
            event,
            enqueued_at_unix_nanos: unix_nanos(),
            sequence: self.state.sequence.fetch_add(1, Ordering::Relaxed),
        };
        let result = if critical {
            self.state.critical_tx.try_send(event)
        } else {
            self.state.normal_tx.try_send(event)
        };
        match result {
            Ok(()) => EnqueueStatus::Queued,
            Err(TrySendError::Full(_)) => {
                if critical {
                    self.state.critical_dropped.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.state.normal_dropped.fetch_add(1, Ordering::Relaxed);
                }
                EnqueueStatus::Dropped
            }
            Err(TrySendError::Disconnected(_)) => EnqueueStatus::Closed,
        }
    }
}
