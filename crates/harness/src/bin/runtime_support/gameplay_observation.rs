// SPDX-License-Identifier: MIT

use serde_json::Value;

pub(super) fn play_card_observation_changed(before: &Value, after: &Value) -> bool {
    let mut changed = false;
    for key in [
        "hand_count",
        "energy",
        "draw_pile_count",
        "discard_pile_count",
        "exhaust_pile_count",
    ] {
        let (Some(before), Some(after)) = (before[key].as_u64(), after[key].as_u64()) else {
            return false;
        };
        changed |= before != after;
    }
    changed
}

pub(super) fn observation_counts(value: &Value) -> Value {
    let fields = [
        "generation",
        "hand_count",
        "energy",
        "draw_pile_count",
        "discard_pile_count",
        "exhaust_pile_count",
    ];
    Value::Object(
        fields
            .into_iter()
            .filter_map(|key| {
                value[key]
                    .as_u64()
                    .map(|number| (key.to_owned(), Value::from(number)))
            })
            .collect(),
    )
}
