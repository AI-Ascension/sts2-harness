// SPDX-License-Identifier: MIT

    use super::{json, legal_action_argument};
    use serde_json::Value;

    #[test]
    fn dispatch_preserves_the_complete_host_legal_action_reference()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut request: Value = serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v3-gameplay/golden/dispatch-action-request.json"
        ))?;
        let schema: Value = serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v3-gameplay/schema.json"
        ))?;
        let validator = jsonschema::validator_for(&schema)?;
        let original_action = request["action"].clone();
        let action_id = original_action["action_id"].as_str().ok_or("action ID")?;
        let payload = original_action["action"].clone();
        request["action"] = legal_action_argument(action_id, payload.clone());
        assert_eq!(request["action"], original_action);
        assert!(validator.is_valid(&request));
        request["action"] = payload;
        assert!(
            !validator.is_valid(&request),
            "bare payload must be rejected"
        );
        let card = json!({"kind": "play_card", "card_id": "c1", "target_id": null});
        request["action"] = legal_action_argument("host-card-ref", card.clone());
        assert_eq!(request["action"]["action_id"], "host-card-ref");
        assert_eq!(request["action"]["action"], card);
        assert!(validator.is_valid(&request));
        Ok(())
    }
