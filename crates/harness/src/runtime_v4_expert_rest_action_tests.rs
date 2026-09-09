// SPDX-License-Identifier: MIT

    use super::{
        RuntimeV4ExpertRestActionParseError, RuntimeV4ExpertRestActionRequest,
        RuntimeV4ExpertRestActionResult, RuntimeV4ExpertRestActionStatus,
    };
    use serde_json::Value;

    const COMPLETED: &str = include_str!(
        "../../../protocol-artifact/runtime-v4-expert-rest-action/golden/action-completed.json"
    );
    const REQUESTED: &str = include_str!(
        "../../../protocol-artifact/runtime-v4-expert-rest-action/golden/action-mend-selection-requested.json"
    );
    const PROGRESSED: &str = include_str!(
        "../../../protocol-artifact/runtime-v4-expert-rest-action/golden/action-selection-progressed.json"
    );

    #[test]
    fn parses_completed_and_selector_transitions() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            RuntimeV4ExpertRestActionResult::parse(COMPLETED.as_bytes())?.status(),
            RuntimeV4ExpertRestActionStatus::Settled
        );
        assert_eq!(
            RuntimeV4ExpertRestActionResult::parse(REQUESTED.as_bytes())?.transition_kind(),
            Some("rest_option_selection_requested")
        );
        assert_eq!(
            RuntimeV4ExpertRestActionResult::parse(PROGRESSED.as_bytes())?.transition_kind(),
            Some("rest_option_selection_progressed")
        );
        Ok(())
    }

    #[test]
    fn duplicate_object_keys_are_rejected() {
        let mut bytes = COMPLETED.as_bytes().to_vec();
        bytes.extend_from_slice(b" ");
        assert!(RuntimeV4ExpertRestActionResult::parse(br#"{"protocol_version":"runtime-v4-expert-rest-action-v1","protocol_version":"runtime-v4-expert-rest-action-v1"}"#).is_err());
        assert!(!bytes.is_empty());
    }

    #[test]
    fn non_settled_result_keeps_the_dispatched_state_identity()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RuntimeV4ExpertRestActionRequest::parse(include_bytes!(
            "../../../protocol-artifact/runtime-v4-expert-rest-action/golden/action-request.json"
        ))
        ?;
        let mut value: Value = serde_json::from_str(include_str!(
            "../../../protocol-artifact/runtime-v4-expert-rest-action/golden/action-unknown.json"
        ))
        ?;
        value["generation"] = Value::from(8_u64);
        let result = RuntimeV4ExpertRestActionResult::from_value(value)?;
        assert_eq!(
            result.matches_request(&request),
            Err(RuntimeV4ExpertRestActionParseError::IdentityMismatch)
        );
        Ok(())
    }
