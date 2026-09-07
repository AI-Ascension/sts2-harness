// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests {
    use super::{
        DecisionKind, DispatchTelemetryStatus, EventKind, FailureCode, GameOutcome,
        ObservationSource, TelemetryContext, TelemetryContextInput, TelemetryEvent, TelemetryStage,
        event_attributes, post_otlp, post_otlp_to, render_span, render_span_at,
        valid_otlp_success_body,
    };
    use serde_json::{Value, json};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn context() -> Result<TelemetryContext, String> {
        TelemetryContext::new(TelemetryContextInput {
            run_id: "run-test",
            episode_id: "episode-test",
            trajectory_id: "trajectory-test",
            trace_id: "trace-test",
            instance_id: "instance-test",
            session_id: "session-test",
            runtime_profile: "runtime-v3-gameplay",
            provider_revision: "7801005e6a1ab77008a05dbba80e0a2a7a56e35d",
        })
    }

    #[test]
    fn context_keeps_lineage_namespaces_distinct() {
        let duplicate = TelemetryContext::new(TelemetryContextInput {
            run_id: "run",
            episode_id: "run",
            trajectory_id: "trajectory",
            trace_id: "trace",
            instance_id: "instance",
            session_id: "session",
            runtime_profile: "profile",
            provider_revision: &"a".repeat(64),
        });
        assert!(duplicate.is_err());
        let invalid_revision = TelemetryContext::new(TelemetryContextInput {
            run_id: "run",
            episode_id: "episode",
            trajectory_id: "trajectory",
            trace_id: "trace",
            instance_id: "instance",
            session_id: "session",
            runtime_profile: "profile",
            provider_revision: "invalid",
        });
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
    fn queued_timestamp_and_sequence_are_rendered() -> Result<(), String> {
        let rendered = render_span_at(&context()?, &TelemetryEvent::RunStarted, 9, 1234);
        assert_eq!(rendered.get("startTimeUnixNano"), Some(&json!("1234")));
        assert_eq!(rendered.get("endTimeUnixNano"), Some(&json!("1234")));
        assert!(rendered["attributes"].as_array().is_some_and(|attributes| {
            attributes.iter().any(|attribute| {
                attribute["key"] == "sts2.event_sequence" && attribute["value"]["stringValue"] == "9"
            })
        }));
        Ok(())
    }

    fn serve_response(response: Vec<u8>) -> Result<bool, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let server = thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(std::time::Duration::from_secs(2)))?;
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request)?;
            stream.write_all(&response)
        });
        let accepted = post_otlp_to(&address, b"{}");
        server
            .join()
            .map_err(|_| String::from("test response server panicked"))?
            .map_err(|error| error.to_string())?;
        Ok(accepted)
    }

    fn framed_response(body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    #[test]
    fn otlp_response_requires_complete_success_body_and_zero_rejections() -> Result<(), String> {
        let cases = [
            ("valid_empty", framed_response("{}"), true),
            (
                "valid_partial_success",
                framed_response(r#"{"partialSuccess":{}}"#),
                true,
            ),
            (
                "valid_string_zero",
                framed_response(r#"{"partialSuccess":{"rejectedSpans":"0"}}"#),
                true,
            ),
            (
                "header_only",
                b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec(),
                false,
            ),
            (
                "missing_content_length",
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{}".to_vec(),
                false,
            ),
            (
                "chunked_framing",
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n".to_vec(),
                false,
            ),
            (
                "truncated_body",
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{".to_vec(),
                false,
            ),
            (
                "malformed_json",
                framed_response(r#"{"partialSuccess":"#),
                false,
            ),
            (
                "string_rejection",
                framed_response(r#"{"partialSuccess":{"rejectedSpans":"1"}}"#),
                false,
            ),
            (
                "numeric_rejection",
                framed_response(r#"{"partialSuccess":{"rejectedSpans":1}}"#),
                false,
            ),
            (
                "invalid_rejection_type",
                framed_response(r#"{"partialSuccess":{"rejectedSpans":"many"}}"#),
                false,
            ),
            (
                "unknown_response_field",
                framed_response(r#"{"unexpected":true}"#),
                false,
            ),
            (
                "trailing_bytes",
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}x".to_vec(),
                false,
            ),
            (
                "unsupported_version",
                framed_response("{}").iter().copied().enumerate().fold(
                    Vec::new(),
                    |mut bytes, (index, byte)| {
                        if index == 7 {
                            bytes.extend_from_slice(b"2");
                        } else {
                            bytes.push(byte);
                        }
                        bytes
                    },
                ),
                false,
            ),
        ];
        for (name, response, expected) in cases {
            assert_eq!(serve_response(response)?, expected, "{name}");
        }
        assert!(valid_otlp_success_body(br#"{"partialSuccess":{"rejectedSpans":0}}"#));
        assert!(!valid_otlp_success_body(br#"{"partialSuccess":{"rejectedSpans":-1}}"#));
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
