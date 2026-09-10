// SPDX-License-Identifier: MIT

use serde_json::Value;
use sts2_harness::{ActionIdentity, EpisodeLegalAction};

#[derive(Clone, Debug)]
pub(super) struct OperationRecord {
    pub(super) state_id: String,
    pub(super) generation: u64,
    pub(super) action: EpisodeLegalAction,
    /// The exact host payload admitted with the first dispatch. Reconciliation must not
    /// consult the mutable current catalog after a reobserve or settlement.
    pub(super) payload: Value,
    /// The selector catalog that admitted a selection action, when one was active. A completion
    /// response can expose only the post-selection rest catalog, so reconciliation must retain
    /// the pre-dispatch selector alongside the operation record.
    pub(super) rest_selector: Option<Value>,
}

impl OperationRecord {
    pub(super) fn new(
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
        payload: Value,
    ) -> Self {
        Self {
            state_id: identity.state_id.clone(),
            generation: identity.generation,
            action: action.clone(),
            payload,
            rest_selector: None,
        }
    }

    pub(super) fn with_rest_selector(mut self, selector: Option<Value>) -> Self {
        self.rest_selector = selector;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::OperationRecord;
    use serde_json::json;
    use sts2_harness::{ActionIdentity, ActionKind, EpisodeLegalAction};

    #[test]
    fn operation_record_keeps_the_original_action_payload() -> Result<(), String> {
        let action = EpisodeLegalAction::new("potion:7:fire", ActionKind::UsePotion)
            .map_err(|error| error.to_string())?;
        let identity =
            ActionIdentity::new("operation:7", "live:7", 7, action.action_id().to_owned())
                .map_err(|error| error.to_string())?;
        let original = json!({
            "kind": "use_potion",
            "potion_id": "potion:fire",
            "target_id": "enemy:1"
        });
        let record = OperationRecord::new(&identity, &action, original.clone());
        assert_eq!(record.payload, original);
        assert_ne!(record.payload, json!({"kind": "use_potion"}));
        Ok(())
    }
}
