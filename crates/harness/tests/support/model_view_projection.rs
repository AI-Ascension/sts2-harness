// SPDX-License-Identifier: MIT

//! Synthetic fixtures shared by the closed model-view projection suites (issue #110).
//!
//! Every fixture is synthetic: no provider, host, or game is contacted.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
#![allow(dead_code)]

use serde_json::{Value, json};
use sts2_harness::context_control::{ModelViewProjection, PathSegment, ViewFieldPath};

pub fn named(name: &str) -> PathSegment {
    PathSegment::named(name)
}

pub fn path(segments: &[PathSegment]) -> ViewFieldPath {
    ViewFieldPath::new(segments.to_vec())
}

/// The smallest recipe that satisfies required-root coverage.
pub fn minimal_recipe() -> ModelViewProjection {
    ModelViewProjection::new(
        "selector-basic",
        "revision-1",
        vec![
            path(&[named("state_id")]),
            path(&[named("generation")]),
            path(&[named("player"), named("hp")]),
            path(&[named("player"), named("max_hp")]),
            path(&[named("player"), named("energy")]),
            path(&[named("player"), named("gold")]),
            path(&[named("player"), named("hand")]),
            path(&[named("state"), named("state")]),
        ],
    )
    .expect("recipe resolves")
}

pub fn observation() -> Value {
    json!({
        "state_id": "state-1",
        "generation": 7,
        "visible_seed": "seed-abc",
        "player": {
            "hp": 40,
            "max_hp": 80,
            "energy": 3,
            "gold": 120,
            "hand": [
                {"card_id": "strike", "name": "Strike", "cost": 1, "upgraded": false},
                {"card_id": "defend", "name": "Defend", "cost": 1, "upgraded": true}
            ],
            "deck": [{"card_id": "bash", "name": "Bash", "cost": 2, "upgraded": false}],
            "discard": [],
            "exhaust": []
        },
        "state": {"state": "combat", "turn_index": 4, "enemies": []},
        "legal_actions": [
            {"action_id": "a1", "action": {"kind": "end_turn"}},
            {"action_id": "a2", "action": {"kind": "play_card", "card_id": "strike", "target_id": "e1"}}
        ]
    })
}
