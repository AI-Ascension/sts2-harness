// SPDX-License-Identifier: MIT

impl RuntimeV4ExpertObservation {
    /// Validates a provider-facing expert projection after its harness-only action forms are
    /// normalized to the frozen native Runtime-v4 expert wire shape. Native callers must use
    /// `from_value`, which continues to reject these projected forms.
    pub(crate) fn validate_fair_play_value(
        mut value: Value,
    ) -> Result<(), RuntimeV4ExpertParseError> {
        let actions = value
            .get_mut("legal_actions")
            .and_then(Value::as_array_mut)
            .ok_or(RuntimeV4ExpertParseError::InvalidShape)?;
        for legal_action in actions {
            let action = legal_action
                .get_mut("action")
                .and_then(Value::as_object_mut)
                .ok_or(RuntimeV4ExpertParseError::InvalidShape)?;
            let kind = action
                .get("kind")
                .and_then(Value::as_str)
                .ok_or(RuntimeV4ExpertParseError::InvalidShape)?;
            match kind {
                "select_card" if action.len() == 2 && action.contains_key("card_id") => {
                    action.insert("selection_id".to_owned(), Value::Null);
                }
                "confirm_selection" | "cancel_selection"
                    if action.len() == 1 && action.contains_key("kind") =>
                {
                    action.insert("selection_id".to_owned(), Value::Null);
                }
                "select_player" if action.len() == 2 && action.contains_key("player_id") => {
                    let player_id = action
                        .get("player_id")
                        .cloned()
                        .ok_or(RuntimeV4ExpertParseError::InvalidShape)?;
                    *action = Map::from_iter([
                        (String::from("kind"), Value::String(String::from("select_card"))),
                        (String::from("selection_id"), Value::Null),
                        (String::from("card_id"), player_id),
                    ]);
                }
                _ => {}
            }
        }
        RuntimeV4ExpertObservation::from_value(value).map(|_| ())
    }
}
