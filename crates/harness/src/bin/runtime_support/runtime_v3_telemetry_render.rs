// SPDX-License-Identifier: MIT

fn render_span(context: &TelemetryContext, event: &TelemetryEvent, sequence: u64) -> Value {
    let (kind, status_error, attrs) = event_attributes(context, event);
    let now = unix_nanos();
    let trace_id = digest("trace", &context.trace_id)[..32].to_owned();
    let root_span_id = digest("root-span", &context.trace_id)[..16].to_owned();
    let span_id = if kind == EventKind::RunStarted {
        root_span_id.clone()
    } else {
        digest(
            "span",
            &format!("{}:{sequence}:{}", context.trace_id, kind.as_str()),
        )[..16]
            .to_owned()
    };
    let attributes = attrs
        .into_iter()
        .map(|(key, value)| json!({"key": key, "value": {"stringValue": value}}))
        .collect::<Vec<_>>();
    let mut span = json!({
        "traceId": trace_id,
        "spanId": span_id,
        "name": format!("sts2.{}", kind.as_str()),
        "startTimeUnixNano": now.to_string(),
        "endTimeUnixNano": now.to_string(),
        "attributes": attributes,
        "status": {"code": if status_error {"STATUS_CODE_ERROR"} else {"STATUS_CODE_UNSET"}}
    });
    if kind != EventKind::RunStarted {
        span["parentSpanId"] = Value::String(root_span_id);
    }
    span
}

fn event_attributes(
    context: &TelemetryContext,
    event: &TelemetryEvent,
) -> (EventKind, bool, Vec<(&'static str, String)>) {
    let mut attrs = vec![
        ("sts2.event", String::new()),
        ("sts2.run_id", context.run_id.clone()),
        ("sts2.episode_id", context.episode_id.clone()),
        ("sts2.trajectory_id", context.trajectory_id.clone()),
        ("sts2.trace_id", context.trace_id.clone()),
        ("sts2.instance_id", context.instance_id.clone()),
        ("sts2.session_id", context.session_id.clone()),
        (
            "sts2.provider_revision_digest",
            context.provider_revision_digest.clone(),
        ),
        ("sts2.schema_version", context.schema_version.clone()),
        ("sts2.runtime_profile", context.runtime_profile.clone()),
    ];
    let kind = match event {
        TelemetryEvent::RunStarted => EventKind::RunStarted,
        TelemetryEvent::ModelDecision {
            model_execution_id,
            decision_kind,
            action_id_digest,
            operation_id_digest,
            confidence,
        } => {
            let kind = EventKind::ModelDecision;
            add(
                &mut attrs,
                "sts2.model_execution_id",
                model_execution_id.to_string(),
            );
            add(
                &mut attrs,
                "sts2.status",
                decision_kind_name(*decision_kind),
            );
            add_optional(&mut attrs, "sts2.action_id_digest", action_id_digest);
            add_optional(&mut attrs, "sts2.operation_id_digest", operation_id_digest);
            if let Some(confidence) = confidence {
                add(&mut attrs, "sts2.confidence", confidence.to_string());
            }
            kind
        }
        TelemetryEvent::ModelFailure {
            model_execution_id,
            failure_code,
        } => {
            add(
                &mut attrs,
                "sts2.model_execution_id",
                model_execution_id.to_string(),
            );
            add(&mut attrs, "sts2.failure_code", failure_code.as_str());
            EventKind::ModelFailure
        }
        TelemetryEvent::Observation {
            source,
            generation,
            stage,
            state_id_digest,
        } => {
            add(&mut attrs, "sts2.source", source.as_str());
            add(&mut attrs, "sts2.generation", generation.to_string());
            add(&mut attrs, "sts2.stage", stage.as_str());
            add(&mut attrs, "sts2.state_id_digest", state_id_digest);
            EventKind::Observation
        }
        TelemetryEvent::ActionDispatch {
            operation_id_digest,
            action_id_digest,
            action_kind,
            generation,
            status,
            failure_code,
        } => {
            add(&mut attrs, "sts2.operation_id_digest", operation_id_digest);
            add(&mut attrs, "sts2.action_id_digest", action_id_digest);
            add(&mut attrs, "sts2.action_kind", action_kind.as_str());
            add(&mut attrs, "sts2.generation", generation.to_string());
            add(&mut attrs, "sts2.status", status.as_str());
            add_optional_code(&mut attrs, "sts2.failure_code", failure_code);
            EventKind::ActionDispatch
        }
        TelemetryEvent::SettlementObservation {
            operation_id_digest,
            action_id_digest,
            from_generation,
            to_generation,
            stage,
            effect_class,
            effect_digest,
            source,
        } => {
            add(&mut attrs, "sts2.operation_id_digest", operation_id_digest);
            add(&mut attrs, "sts2.action_id_digest", action_id_digest);
            add(
                &mut attrs,
                "sts2.from_generation",
                from_generation.to_string(),
            );
            add(&mut attrs, "sts2.to_generation", to_generation.to_string());
            add(&mut attrs, "sts2.stage", stage.as_str());
            add(&mut attrs, "sts2.effect_class", *effect_class);
            add(&mut attrs, "sts2.effect_digest", effect_digest);
            add(&mut attrs, "sts2.source", source.as_str());
            EventKind::SettlementObservation
        }
        TelemetryEvent::Recovery {
            kind,
            operation_id_digest,
            attempt,
            outcome,
            failure_code,
        } => {
            add(&mut attrs, "sts2.recovery_kind", kind.as_str());
            add_optional(&mut attrs, "sts2.operation_id_digest", operation_id_digest);
            add(&mut attrs, "sts2.recovery_attempt", attempt.to_string());
            add(&mut attrs, "sts2.status", *outcome);
            add_optional_code(&mut attrs, "sts2.failure_code", failure_code);
            EventKind::Recovery
        }
        TelemetryEvent::Failure {
            boundary,
            failure_code,
            retryable,
            operation_id_digest,
        } => {
            add(&mut attrs, "sts2.boundary", *boundary);
            add(&mut attrs, "sts2.failure_code", failure_code.as_str());
            add(&mut attrs, "sts2.retryable", bool_string(*retryable));
            add_optional(&mut attrs, "sts2.operation_id_digest", operation_id_digest);
            EventKind::Failure
        }
        TelemetryEvent::TerminalObserved {
            stage,
            outcome,
            generation,
            state_id_digest,
        } => {
            add(&mut attrs, "sts2.stage", stage.as_str());
            add(&mut attrs, "sts2.game_outcome", outcome.as_str());
            add(&mut attrs, "sts2.generation", generation.to_string());
            add(&mut attrs, "sts2.state_id_digest", state_id_digest);
            EventKind::TerminalObserved
        }
        TelemetryEvent::RunFinished {
            outcome,
            terminal_stage,
            cleanup_status,
            dropped_events,
        } => {
            add(&mut attrs, "sts2.game_outcome", outcome.as_str());
            add(&mut attrs, "sts2.stage", terminal_stage.as_str());
            add(&mut attrs, "sts2.cleanup_status", cleanup_status.as_str());
            add(
                &mut attrs,
                "sts2.dropped_events",
                dropped_events.to_string(),
            );
            add(&mut attrs, "sts2.export_status", "queued");
            EventKind::RunFinished
        }
    };
    attrs[0].1 = kind.as_str().to_owned();
    let status_error = matches!(kind, EventKind::ModelFailure | EventKind::Failure);
    (kind, status_error, attrs)
}

fn add(attrs: &mut Vec<(&'static str, String)>, key: &'static str, value: impl Into<String>) {
    attrs.push((key, value.into()));
}

fn add_optional(
    attrs: &mut Vec<(&'static str, String)>,
    key: &'static str,
    value: &Option<String>,
) {
    if let Some(value) = value {
        add(attrs, key, value);
    }
}

fn add_optional_code(
    attrs: &mut Vec<(&'static str, String)>,
    key: &'static str,
    value: &Option<FailureCode>,
) {
    if let Some(value) = value {
        add(attrs, key, value.as_str());
    }
}

fn decision_kind_name(kind: DecisionKind) -> &'static str {
    match kind {
        DecisionKind::Action => "action",
        DecisionKind::Plan => "plan",
        DecisionKind::Wait => "wait",
        DecisionKind::Reobserve => "reobserve",
        DecisionKind::Recovery => "recovery",
    }
}

fn bool_string(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn effect_class(value: &str) -> &'static str {
    match value {
        "card_play" | "card_played" => "card_play",
        "end_turn" | "turn_ended" => "end_turn",
        "reward" | "reward_selected" => "reward",
        "shop_purchase" | "shop_remove" => "shop",
        "map" | "map_node_selected" => "map",
        "victory" | "defeat" => "terminal",
        _ => "other",
    }
}

fn digest(domain: &str, value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn valid_revision(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn safe_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos())
}
