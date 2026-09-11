// SPDX-License-Identifier: MIT

#[cfg(test)]
mod identity_privacy_tests {
    use super::{
        DecisionKind, DispatchStatus, DispatchTelemetryStatus, GameOutcome, TelemetryActionKind,
        TelemetryContext, TelemetryContextInput, TelemetryEvent, TelemetryHandle, TelemetryStage,
        digest, event_attributes, render_span, serialize_otlp_batch,
    };
    use serde_json::Value;
    use sts2_harness::ActionKind;

    fn assert_no_string_value_contains(value: &Value, forbidden: &str) {
        match value {
            Value::Array(values) => {
                for value in values {
                    assert_no_string_value_contains(value, forbidden);
                }
            }
            Value::Object(values) => {
                for value in values.values() {
                    assert_no_string_value_contains(value, forbidden);
                }
            }
            Value::String(value) => assert!(
                !value.contains(forbidden),
                "serialized OTLP value leaked {forbidden}"
            ),
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }

    #[test]
    fn rendered_identity_attributes_are_domain_separated_digests() -> Result<(), String> {
        let raw = [
            ("sts2.run_id", "RAW_RUN_ID_SENTINEL", "run"),
            ("sts2.episode_id", "RAW_EPISODE_ID_SENTINEL", "episode"),
            ("sts2.trajectory_id", "RAW_TRAJECTORY_ID_SENTINEL", "trajectory"),
            ("sts2.trace_id", "RAW_TRACE_ID_SENTINEL", "trace"),
            ("sts2.instance_id", "RAW_INSTANCE_ID_SENTINEL", "instance"),
            ("sts2.session_id", "RAW_SESSION_ID_SENTINEL", "session"),
        ];
        let context = TelemetryContext::new(TelemetryContextInput {
            run_id: raw[0].1,
            episode_id: raw[1].1,
            trajectory_id: raw[2].1,
            trace_id: raw[3].1,
            instance_id: raw[4].1,
            session_id: raw[5].1,
            runtime_profile: "runtime-v3-gameplay",
            provider_revision: "7801005e6a1ab77008a05dbba80e0a2a7a56e35d",
        })?;
        let root = render_span(&context, &TelemetryEvent::RunStarted, 1);
        let rendered = root.to_string();
        let (_, _, attributes) = event_attributes(&context, &TelemetryEvent::RunStarted);

        for (attribute, identity, domain) in raw {
            let value = attributes
                .iter()
                .find_map(|(key, value)| (*key == attribute).then_some(value))
                .ok_or_else(|| format!("missing {attribute}"))?;
            assert_eq!(value, &digest(domain, identity));
            assert_eq!(value.len(), 64);
            assert!(value.bytes().all(|byte| byte.is_ascii_hexdigit()));
            assert!(!rendered.contains(identity), "serialized {attribute} leaked raw identity");
        }
        let expected_trace = digest("trace", raw[3].1);
        assert_eq!(root["traceId"].as_str(), Some(&expected_trace[..32]));
        assert_ne!(digest("run", "RAW_SHARED_ID_SENTINEL"), digest("episode", "RAW_SHARED_ID_SENTINEL"));
        assert!(rendered.contains("\"sts2.id_encoding\""));
        assert!(rendered.contains("\"digest\""));
        assert!(!rendered.contains("RAW_"));

        let action = render_span(
            &context,
            &TelemetryEvent::ActionDispatch {
                operation_id_digest: digest("operation", "RAW_OPERATION_ID_SENTINEL"),
                action_id_digest: digest("action", "RAW_ACTION_ID_SENTINEL"),
                action_kind: TelemetryActionKind::EndTurn,
                generation: 1,
                status: DispatchTelemetryStatus::Accepted,
                failure_code: None,
            },
            2,
        )
        .to_string();
        assert!(!action.contains("RAW_OPERATION_ID_SENTINEL"));
        assert!(!action.contains("RAW_ACTION_ID_SENTINEL"));
        assert!(action.contains(&digest("operation", "RAW_OPERATION_ID_SENTINEL")));
        assert!(action.contains(&digest("action", "RAW_ACTION_ID_SENTINEL")));
        Ok(())
    }

    #[test]
    fn public_ingress_redacts_adversarial_values_from_full_otlp_terminal_envelope(
    ) -> Result<(), String> {
        let sentinels = [
            "PRIVATE_PROMPT_SENTINEL",
            "MODEL_OUTPUT_SENTINEL",
            "BEARER_TOKEN_SENTINEL",
            r"C:\\Users\\private\\save",
            "PROPRIETARY_HOST_TEXT_SENTINEL",
        ];
        let raw_operation = sentinels.join("|");
        let raw_action = format!("action:{}", sentinels.join("|"));
        let context = TelemetryContext::new(TelemetryContextInput {
            run_id: "run-privacy-test",
            episode_id: "episode-privacy-test",
            trajectory_id: "trajectory-privacy-test",
            trace_id: "trace-privacy-test",
            instance_id: "instance-privacy-test",
            session_id: "session-privacy-test",
            runtime_profile: "runtime-v3-gameplay",
            provider_revision: "7801005e6a1ab77008a05dbba80e0a2a7a56e35d",
        })?;
        let expected_action_digest = digest("action", &raw_action);
        let expected_operation_digest = digest("operation", &raw_operation);
        let (handle, receiver) = TelemetryHandle::with_test_sink(context.clone());

        assert_eq!(
            handle.model_decision(
                9,
                DecisionKind::Action,
                Some(&raw_action),
                Some(&raw_operation),
                Some(80),
            ),
            super::EnqueueStatus::Queued
        );
        assert_eq!(
            handle.action_dispatch(
                &raw_operation,
                &raw_action,
                ActionKind::EndTurn,
                7,
                DispatchStatus::Accepted,
                None,
            ),
            super::EnqueueStatus::Queued
        );
        assert_eq!(
            handle.run_finished(
                GameOutcome::Failure,
                TelemetryStage::Defeat,
                super::CleanupStatus::Clean,
            ),
            super::EnqueueStatus::Queued
        );

        let events = (0..3)
            .map(|_| receiver.try_recv().map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let envelope = serialize_otlp_batch(&context, &events)
            .ok_or_else(|| String::from("telemetry batch serialization failed"))?;
        let serialized = String::from_utf8(envelope).map_err(|error| error.to_string())?;
        let decoded: Value = serde_json::from_str(&serialized).map_err(|error| error.to_string())?;

        for sentinel in sentinels {
            assert_no_string_value_contains(&decoded, sentinel);
        }
        assert!(serialized.contains(&expected_action_digest));
        assert!(serialized.contains(&expected_operation_digest));
        assert!(serialized.contains("sts2.run_finished"));
        assert!(serialized.contains("sts2.id_encoding"));
        Ok(())
    }
}
