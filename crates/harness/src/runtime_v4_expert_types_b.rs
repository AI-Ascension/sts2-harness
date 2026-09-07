// SPDX-License-Identifier: MIT


#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields, tag = "kind")]
enum IntentComponent {
    #[serde(rename = "attack")]
    Attack {
        damage: u16,
        hits: u8,
        target_ids: Option<Vec<String>>,
    },
    #[serde(rename = "defend")]
    Defend { target_ids: Option<Vec<String>> },
    #[serde(rename = "buff")]
    Buff { target_ids: Option<Vec<String>> },
    #[serde(rename = "debuff")]
    Debuff { target_ids: Option<Vec<String>> },
    #[serde(rename = "unknown")]
    Unknown { target_ids: Option<Vec<String>> },
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields, tag = "state")]
enum State {
    #[serde(rename = "setup")]
    Setup { characters: Vec<String> },
    #[serde(rename = "map")]
    Map {
        current_node_id: Option<String>,
        nodes: Option<Vec<MapNode>>,
        edges: Option<Vec<MapEdge>>,
        options: Vec<String>,
    },
    #[serde(rename = "combat")]
    Combat {
        turn_index: u16,
        enemies: Option<Vec<Enemy>>,
    },
    #[serde(rename = "reward")]
    Reward { choices: Option<Vec<Choice>> },
    #[serde(rename = "event")]
    Event { choices: Option<Vec<Choice>> },
    #[serde(rename = "rest")]
    Rest { choices: Option<Vec<Choice>> },
    #[serde(rename = "selection")]
    Selection { choices: Option<Vec<Choice>> },
    #[serde(rename = "shop")]
    Shop { items: Option<Vec<ShopItem>> },
    #[serde(rename = "victory")]
    Victory,
    #[serde(rename = "defeat")]
    Defeat { reason: Option<String> },
    #[serde(rename = "recovery")]
    Recovery { code: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct MapNode {
    node_id: String,
    act: u8,
    row: u8,
    col: u8,
    kind: String,
    reachable: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct MapEdge {
    from: String,
    to: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct Choice {
    choice_id: String,
    label: String,
    kind: String,
    domain: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct ShopItem {
    item_id: String,
    name: String,
    kind: String,
    price: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct LegalAction {
    action_id: String,
    action: Action,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields, tag = "kind")]
enum Action {
    #[serde(rename = "start_run")]
    StartRun { character_id: String },
    #[serde(rename = "select_character")]
    SelectCharacter { character_id: String },
    #[serde(rename = "select_map_node")]
    SelectMapNode { node_id: String },
    #[serde(rename = "play_card")]
    PlayCard {
        card_id: String,
        target_id: Option<String>,
    },
    #[serde(rename = "use_potion")]
    UsePotion {
        potion_id: String,
        target_id: Option<String>,
    },
    #[serde(rename = "end_turn")]
    EndTurn,
    #[serde(rename = "skip_reward")]
    SkipReward,
    #[serde(rename = "proceed")]
    Proceed,
    #[serde(rename = "rest")]
    Rest,
    #[serde(rename = "confirm_victory")]
    ConfirmVictory,
    #[serde(rename = "save_quit")]
    SaveQuit,
    #[serde(rename = "rest_option")]
    RestOption { rest_option_id: String },
    #[serde(rename = "choose_reward")]
    ChooseReward { reward_id: String },
    #[serde(rename = "shop_purchase")]
    ShopPurchase { item_id: String },
    #[serde(rename = "shop_remove")]
    ShopRemove { card_id: String },
    #[serde(rename = "smith")]
    Smith { card_id: String },
    #[serde(rename = "event_choice")]
    EventChoice { choice_id: String },
    #[serde(rename = "select_card")]
    SelectCard {
        selection_id: Option<String>,
        card_id: String,
    },
    #[serde(rename = "confirm_selection")]
    ConfirmSelection { selection_id: Option<String> },
    #[serde(rename = "cancel_selection")]
    CancelSelection { selection_id: Option<String> },
}
