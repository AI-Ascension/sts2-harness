// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests {
    use super::{
        DecisionKind, DispatchTelemetryStatus, EventKind, FailureCode, GameOutcome,
        ObservationSource, TelemetryContext, TelemetryEvent, TelemetryStage, event_attributes,
        post_otlp, render_span,
    };
    use serde_json::{Value, json};

    fn context() -> Result<TelemetryContext, String> {
        TelemetryContext::new(
            "run-test",
            "episode-test",
            "trajectory-test",
            "trace-test",
            "instance-test",
            "session-test",
            "runtime-v3-gameplay",
            "7801005e6a1ab77008a05dbba80e0a2a7a56e35d",
        )
    }

    #[test]
    fn context_keeps_lineage_namespaces_distinct() {
        let duplicate = TelemetryContext::new(
            "run",
            "run",
            "trajectory",
            "trace",
            "instance",
            "session",
            "profile",
            &"a".repeat(64),
        );
        assert!(duplicate.is_err());
        let invalid_revision = TelemetryContext::new(
            "run",
            "episode",
            "trajectory",
            "trace",
            "instance",
            "session",
            "profile",
            "invalid",
        );
        assert!(invalid_revision.is_err());
    }

    #[test]
    fn rendered_decision_contains_only_allowlisted_private_safe_fields() -> Result<(), String> {
        let event = TelemetryEvent::ModelDecision {
            model_execution_id: 7,
            decision_kind: DecisionKind::Action,
            action_id_digest: Some(String::from("abc")),
            operation_id_digest: None,
            confidence: Some(90),
        };
        let body = render_span(&context()?, &event, 1).to_string();
        assert!(!body.contains("SENTINEL_PRIVATE_PROMPT"));
        assert!(!body.contains("rationale"));
        assert!(!body.contains("model_output"));
        assert!(body.contains("sts2.model_execution_id"));
        assert!(body.contains("sts2.action_id_digest"));
        assert!(body.contains("sts2.instance_id"));
        assert!(body.contains("sts2.session_id"));
        Ok(())
    }

    #[test]
    fn settlement_is_a_distinct_typed_event() -> Result<(), String> {
        let event = TelemetryEvent::SettlementObservation {
            operation_id_digest: String::from("operation-digest"),
            action_id_digest: String::from("action-digest"),
            from_generation: 4,
            to_generation: 5,
            stage: TelemetryStage::Combat,
            effect_class: "card_play",
            effect_digest: String::from("effect-digest"),
            source: ObservationSource::Transition,
        };
        let (kind, error, attrs) = event_attributes(&context()?, &event);
        assert_eq!(kind, EventKind::SettlementObservation);
        assert!(!error);
        assert!(
            attrs
                .iter()
                .any(|(key, value)| *key == "sts2.from_generation" && value == "4")
        );
        assert!(
            attrs
                .iter()
                .any(|(key, value)| *key == "sts2.to_generation" && value == "5")
        );
        Ok(())
    }

    #[test]
    fn status_and_outcome_are_finite_strings() -> Result<(), String> {
        assert_eq!(DispatchTelemetryStatus::Settled.as_str(), "settled");
        assert_eq!(
            FailureCode::ProviderUnavailable.as_str(),
            "provider_unavailable"
        );
        assert_eq!(GameOutcome::Success.as_str(), "success");
        assert_eq!(ObservationSource::Recovery.as_str(), "recovery");
        let rendered = render_span(&context()?, &TelemetryEvent::RunStarted, 2);
        assert!(
            rendered
                .get("traceId")
                .and_then(Value::as_str)
                .is_some_and(|value| value.len() == 32)
        );
        assert!(
            rendered
                .get("spanId")
                .and_then(Value::as_str)
                .is_some_and(|value| value.len() == 16)
        );
        assert!(rendered.get("parentSpanId").is_none());
        let child = render_span(
            &context()?,
            &TelemetryEvent::Observation {
                source: ObservationSource::Observe,
                generation: 1,
                stage: TelemetryStage::Setup,
                state_id_digest: String::from("state-digest"),
            },
            3,
        );
        assert_eq!(child.get("parentSpanId"), rendered.get("spanId"));
        Ok(())
    }

    #[test]
    fn collector_encoding_smoke_is_opt_in() -> Result<(), String> {
        if std::env::var("STS2_TELEMETRY_COLLECTOR_SMOKE").as_deref() != Ok("true") {
            return Ok(());
        }
        let context = context()?;
        let root = render_span(&context, &TelemetryEvent::RunStarted, 1);
        let child = render_span(
            &context,
            &TelemetryEvent::ModelDecision {
                model_execution_id: 1,
                decision_kind: DecisionKind::Action,
                action_id_digest: Some(String::from("a")),
                operation_id_digest: None,
                confidence: Some(80),
            },
            2,
        );
        let body = serde_json::to_vec(&json!({
            "resourceSpans": [{
                "resource": {"attributes": [
                    {"key": "service.name", "value": {"stringValue": "sts2-harness"}},
                    {"key": "service.version", "value": {"stringValue": "runtime-v3"}},
                    {"key": "deployment.environment", "value": {"stringValue": "local"}}
                ]},
                "scopeSpans": [{
                    "scope": {"name": "sts2.runtime.telemetry", "version": "1"},
                    "spans": [root, child]
                }]
            }]
        }))
        .map_err(|error| error.to_string())?;
        assert!(post_otlp(&body));
        Ok(())
    }
}
