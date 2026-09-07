// SPDX-License-Identifier: MIT

fn shape_is_closed(value: &Value) -> bool {
    let Some(root) = exact_object(
        Some(value),
        &[
            "protocol_version",
            "schema_digest",
            "provenance",
            "profile",
            "state_id",
            "generation",
            "visible_seed",
            "run",
            "player",
            "state",
            "legal_actions",
        ],
    ) else {
        return false;
    };
    let Some(provenance) =
        exact_object(root.get("provenance"), &["artifact", "source", "generator"])
    else {
        return false;
    };
    let _ = provenance;
    let Some(run) = exact_object(root.get("run"), &["character_id", "act", "location"]) else {
        return false;
    };
    let _ = run;
    let Some(player) = exact_object(
        root.get("player"),
        &[
            "hp",
            "max_hp",
            "block",
            "energy",
            "gold",
            "hand",
            "deck",
            "discard",
            "exhaust",
            "powers",
            "statuses",
            "relics",
            "potions",
            "potion_slots",
            "max_potion_slots",
        ],
    ) else {
        return false;
    };
    for field in ["hand", "deck", "discard", "exhaust"] {
        if !shape_cards(player.get(field)) {
            return false;
        }
    }
    for field in ["powers", "statuses"] {
        if !shape_statuses(player.get(field)) {
            return false;
        }
    }
    if !shape_relics(player.get("relics")) || !shape_potions(player.get("potions")) {
        return false;
    }
    if !shape_state(root.get("state")) || !shape_legal_actions(root.get("legal_actions")) {
        return false;
    }
    true
}
fn exact_object<'a>(value: Option<&'a Value>, fields: &[&str]) -> Option<&'a Map<String, Value>> {
    let object = value?.as_object()?;
    (object.len() == fields.len() && fields.iter().all(|field| object.contains_key(*field)))
        .then_some(object)
}
