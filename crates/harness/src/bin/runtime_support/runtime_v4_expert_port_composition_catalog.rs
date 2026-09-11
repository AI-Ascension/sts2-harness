// SPDX-License-Identifier: MIT

pub(super) fn composed_catalog(
    actions: &EpisodeLegalActionSet,
    payloads: &BTreeMap<String, Value>,
) -> Result<(Value, Vec<u8>), String> {
    let catalog = Value::Array(
        actions
            .actions()
            .iter()
            .map(|action| {
                let payload = payloads
                    .get(action.action_id())
                    .ok_or_else(|| String::from("expert composition catalog omitted an action payload"))?;
                if payload.get("kind").and_then(Value::as_str)
                    != Some(wire::action_kind_name(action.kind()))
                {
                    return Err(String::from(
                        "expert composition catalog action kind does not match the action set",
                    ));
                }
                Ok(json!({
                    "action_id": action.action_id(),
                    "action": payload,
                }))
            })
            .collect::<Result<Vec<_>, String>>()?,
    );
    let raw = serde_json::to_vec(&catalog)
        .map_err(|error| format!("expert composition catalog is not encodable: {error}"))?;
    if raw.is_empty() || raw.len() > sts2_harness::MAX_CATALOG_BYTES {
        return Err(String::from("expert composition catalog is invalid"));
    }
    Ok((catalog, raw))
}
