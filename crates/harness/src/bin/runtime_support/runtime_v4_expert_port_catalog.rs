// SPDX-License-Identifier: MIT

fn composed_catalog_from_actions(
    actions: &EpisodeLegalActionSet,
    payloads: &BTreeMap<String, Value>,
) -> Result<Value, String> {
    let values = actions
        .actions()
        .iter()
        .map(|action| {
            let payload = payloads
                .get(action.action_id())
                .ok_or_else(|| format!("missing payload for expert action {}", action.action_id()))?;
            Ok(json!({
                "action_id": action.action_id(),
                "action": payload,
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Value::Array(values))
}
