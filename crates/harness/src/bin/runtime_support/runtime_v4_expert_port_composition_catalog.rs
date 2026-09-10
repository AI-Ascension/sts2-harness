// SPDX-License-Identifier: MIT

fn composed_catalog(observation: &EpisodeObservation) -> Result<(Value, Vec<u8>), String> {
    let catalog = observation
        .fair_play()
        .as_value()
        .get("legal_actions")
        .cloned()
        .ok_or_else(|| String::from("expert composition omitted legal actions"))?;
    let raw = serde_json::to_vec(&catalog)
        .map_err(|error| format!("expert composition catalog is not encodable: {error}"))?;
    if !catalog.is_array() || raw.is_empty() || raw.len() > sts2_harness::MAX_CATALOG_BYTES {
        return Err(String::from("expert composition catalog is invalid"));
    }
    Ok((catalog, raw))
}
