// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::runtime_v4_expert_artifact::{
    RUNTIME_V4_EXPERT_ARTIFACT, RUNTIME_V4_EXPERT_GENERATOR, RUNTIME_V4_EXPERT_PROTOCOL_VERSION,
    RUNTIME_V4_EXPERT_SCHEMA_DIGEST, RUNTIME_V4_EXPERT_SCHEMA_SOURCE,
    verify_runtime_v4_expert_artifact,
};

const MAX_OBSERVATION_BYTES: usize = 128 * 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_TEXT_BYTES: usize = 512;
const MAX_ITEMS: usize = 256;
const MAX_EDGES: usize = 512;
const MAX_TARGETS: usize = 16;

/// A parsed, host-produced Runtime-v4 expert observation.
///
/// The raw value is retained because the existing Exo request contract is JSON based. The typed
/// wire value is private and is validated before the value can cross the provider boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeV4ExpertObservation {
    value: Value,
    wire: WireObservation,
}

impl RuntimeV4ExpertObservation {
    /// Parses and validates one serialized gateway/MCP response.
    pub fn parse(bytes: &[u8]) -> Result<Self, RuntimeV4ExpertParseError> {
        if bytes.len() > MAX_OBSERVATION_BYTES {
            return Err(RuntimeV4ExpertParseError::TooLarge);
        }
        let value: Value =
            serde_json::from_slice(bytes).map_err(|_| RuntimeV4ExpertParseError::MalformedJson)?;
        Self::from_value(value)
    }

    /// Validates an already decoded gateway/MCP response.
    pub fn from_value(value: Value) -> Result<Self, RuntimeV4ExpertParseError> {
        let encoded =
            serde_json::to_vec(&value).map_err(|_| RuntimeV4ExpertParseError::MalformedJson)?;
        if encoded.len() > MAX_OBSERVATION_BYTES {
            return Err(RuntimeV4ExpertParseError::TooLarge);
        }
        if !shape_is_closed(&value) {
            return Err(RuntimeV4ExpertParseError::InvalidShape);
        }
        let wire: WireObservation = serde_json::from_value(value.clone())
            .map_err(|_| RuntimeV4ExpertParseError::InvalidShape)?;
        validate_wire(&wire)?;
        verify_runtime_v4_expert_artifact().map_err(RuntimeV4ExpertParseError::ArtifactMismatch)?;
        Ok(Self { value, wire })
    }

    #[must_use]
    pub fn as_value(&self) -> &Value {
        &self.value
    }

    #[must_use]
    pub fn state_id(&self) -> &str {
        &self.wire.state_id
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.wire.generation
    }

    pub fn legal_action_ids(&self) -> impl Iterator<Item = &str> {
        self.wire
            .legal_actions
            .iter()
            .map(|action| action.action_id.as_str())
    }

    /// Returns the provider-facing observation after the same fair-play validation used by Exo.
    /// This method is useful to callers that already have a decoded v4 response and want to
    /// construct the ordinary provider request prompt.
    pub fn into_sanitized(self) -> Result<crate::SanitizedObservation, crate::SandboxError> {
        crate::SanitizedObservation::new(self.value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeV4ExpertParseError {
    TooLarge,
    MalformedJson,
    InvalidShape,
    InvalidValue,
    ArtifactMismatch(crate::RuntimeV4ExpertArtifactError),
}

impl std::fmt::Display for RuntimeV4ExpertParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::TooLarge => "runtime-v4-expert observation exceeds its byte bound",
            Self::MalformedJson => "runtime-v4-expert observation is malformed JSON",
            Self::InvalidShape => "runtime-v4-expert observation has an invalid closed shape",
            Self::InvalidValue => "runtime-v4-expert observation has an invalid value",
            Self::ArtifactMismatch(_) => "runtime-v4-expert artifact verification failed",
        })
    }
}

impl std::error::Error for RuntimeV4ExpertParseError {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct WireObservation {
    protocol_version: String,
    schema_digest: String,
    provenance: Provenance,
    profile: String,
    state_id: String,
    generation: u64,
    visible_seed: Option<String>,
    run: Run,
    player: Player,
    state: State,
    legal_actions: Vec<LegalAction>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct Provenance {
    artifact: String,
    source: String,
    generator: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct Run {
    character_id: Option<String>,
    act: Option<u8>,
    location: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct Status {
    status_id: String,
    name: String,
    amount: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct Relic {
    relic_id: String,
    name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct Potion {
    potion_id: String,
    name: String,
    slot: Option<u8>,
    usable: Option<bool>,
    target_mode: PotionTargetMode,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum PotionTargetMode {
    #[serde(rename = "self")]
    SelfTarget,
    AnyEnemy,
    AllEnemies,
    None,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Card {
    card_id: String,
    name: String,
    cost: Option<u8>,
    upgraded: bool,
    card_type: Option<String>,
    rarity: Option<String>,
    target: Option<String>,
    description: Option<String>,
}

// The wire field is named `type`, which cannot be used as a Rust identifier.
impl<'de> Deserialize<'de> for Card {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct CardWire {
            card_id: String,
            name: String,
            cost: Option<u8>,
            upgraded: bool,
            #[serde(rename = "type")]
            card_type: Option<String>,
            rarity: Option<String>,
            target: Option<String>,
            description: Option<String>,
        }
        let card = CardWire::deserialize(deserializer)?;
        Ok(Self {
            card_id: card.card_id,
            name: card.name,
            cost: card.cost,
            upgraded: card.upgraded,
            card_type: card.card_type,
            rarity: card.rarity,
            target: card.target,
            description: card.description,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct Player {
    hp: u16,
    max_hp: u16,
    block: Option<u16>,
    energy: u8,
    gold: u64,
    hand: Option<Vec<Card>>,
    deck: Option<Vec<Card>>,
    discard: Option<Vec<Card>>,
    exhaust: Option<Vec<Card>>,
    powers: Option<Vec<Status>>,
    statuses: Option<Vec<Status>>,
    relics: Option<Vec<Relic>>,
    potions: Option<Vec<Potion>>,
    potion_slots: Option<u8>,
    max_potion_slots: Option<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct Enemy {
    enemy_id: String,
    name: String,
    hp: u16,
    max_hp: u16,
    block: Option<u16>,
    powers: Option<Vec<Status>>,
    statuses: Option<Vec<Status>>,
    intent: Intent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind")]
enum Intent {
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

fn validate_wire(wire: &WireObservation) -> Result<(), RuntimeV4ExpertParseError> {
    if wire.protocol_version != RUNTIME_V4_EXPERT_PROTOCOL_VERSION
        || wire.schema_digest != RUNTIME_V4_EXPERT_SCHEMA_DIGEST
        || wire.profile != "expert-state"
        || wire.provenance.artifact != RUNTIME_V4_EXPERT_ARTIFACT
        || wire.provenance.source != RUNTIME_V4_EXPERT_SCHEMA_SOURCE
        || wire.provenance.generator != RUNTIME_V4_EXPERT_GENERATOR
        || wire.generation > MAX_SAFE_INTEGER
        || !valid_identity(&wire.state_id)
        || !valid_identity(&wire.provenance.artifact)
        || !valid_text(&wire.provenance.source)
        || !valid_text(&wire.provenance.generator)
        || wire
            .visible_seed
            .as_deref()
            .is_some_and(|value| !valid_text(value))
        || !valid_run(&wire.run)
        || !valid_player(&wire.player)
        || !valid_state(&wire.state)
        || wire.legal_actions.len() > MAX_ITEMS
    {
        return Err(RuntimeV4ExpertParseError::InvalidValue);
    }
    let mut action_ids = BTreeSet::new();
    for legal_action in &wire.legal_actions {
        if !valid_identity(&legal_action.action_id)
            || !action_ids.insert(legal_action.action_id.as_str())
            || !valid_action(&legal_action.action)
        {
            return Err(RuntimeV4ExpertParseError::InvalidValue);
        }
    }
    Ok(())
}

fn valid_run(run: &Run) -> bool {
    run.character_id.as_deref().is_none_or(valid_identity)
        && run.location.as_deref().is_none_or(valid_identity)
}

fn valid_player(player: &Player) -> bool {
    player.hp <= player.max_hp
        && player.gold <= 4_294_967_295
        && valid_cards(player.hand.as_deref())
        && valid_cards(player.deck.as_deref())
        && valid_cards(player.discard.as_deref())
        && valid_cards(player.exhaust.as_deref())
        && valid_statuses(player.powers.as_deref())
        && valid_statuses(player.statuses.as_deref())
        && valid_relics(player.relics.as_deref())
        && valid_potions(player.potions.as_deref())
}

fn valid_cards(cards: Option<&[Card]>) -> bool {
    cards.is_none_or(|cards| cards.len() <= MAX_ITEMS && cards.iter().all(valid_card))
}

fn valid_card(card: &Card) -> bool {
    valid_identity(&card.card_id)
        && valid_text(&card.name)
        && card.card_type.as_deref().is_none_or(valid_identity)
        && card.rarity.as_deref().is_none_or(valid_identity)
        && card.target.as_deref().is_none_or(valid_identity)
        && card.description.as_deref().is_none_or(valid_text)
}

fn valid_statuses(statuses: Option<&[Status]>) -> bool {
    statuses.is_none_or(|statuses| {
        statuses.len() <= MAX_ITEMS
            && statuses.iter().all(|status| {
                valid_identity(&status.status_id)
                    && valid_text(&status.name)
                    && status
                        .amount
                        .is_none_or(|amount| (-65_535..=65_535).contains(&amount))
            })
    })
}

fn valid_relics(relics: Option<&[Relic]>) -> bool {
    relics.is_none_or(|relics| {
        relics.len() <= MAX_ITEMS
            && relics
                .iter()
                .all(|relic| valid_identity(&relic.relic_id) && valid_text(&relic.name))
    })
}

fn valid_potions(potions: Option<&[Potion]>) -> bool {
    potions.is_none_or(|potions| {
        potions.len() <= MAX_ITEMS
            && potions
                .iter()
                .all(|potion| valid_identity(&potion.potion_id) && valid_text(&potion.name))
    })
}

fn valid_state(state: &State) -> bool {
    match state {
        State::Setup { characters } => valid_identities(characters),
        State::Map {
            current_node_id,
            nodes,
            edges,
            options,
        } => {
            current_node_id.as_deref().is_none_or(valid_identity)
                && valid_map_nodes(nodes.as_deref())
                && edges.as_ref().is_none_or(|edges| {
                    edges.len() <= MAX_EDGES
                        && edges
                            .iter()
                            .all(|edge| valid_identity(&edge.from) && valid_identity(&edge.to))
                })
                && valid_identities(options)
        }
        State::Combat {
            enemies,
            turn_index: _,
        } => valid_enemies(enemies.as_deref()),
        State::Reward { choices }
        | State::Event { choices }
        | State::Rest { choices }
        | State::Selection { choices } => valid_choices(choices.as_deref()),
        State::Shop { items } => items.as_ref().is_none_or(|items| {
            items.len() <= MAX_ITEMS
                && items.iter().all(|item| {
                    valid_identity(&item.item_id)
                        && valid_text(&item.name)
                        && valid_identity(&item.kind)
                })
        }),
        State::Victory => true,
        State::Defeat { reason } => reason.as_deref().is_none_or(valid_text),
        State::Recovery { code } => valid_identity(code),
    }
}

fn valid_map_nodes(nodes: Option<&[MapNode]>) -> bool {
    nodes.is_none_or(|nodes| {
        nodes.len() <= MAX_ITEMS
            && nodes
                .iter()
                .all(|node| valid_identity(&node.node_id) && valid_identity(&node.kind))
    })
}

fn valid_enemies(enemies: Option<&[Enemy]>) -> bool {
    enemies.is_none_or(|enemies| {
        enemies.len() <= MAX_ITEMS
            && enemies.iter().all(|enemy| {
                valid_identity(&enemy.enemy_id)
                    && valid_text(&enemy.name)
                    && enemy.hp <= enemy.max_hp
                    && valid_statuses(enemy.powers.as_deref())
                    && valid_statuses(enemy.statuses.as_deref())
                    && valid_intent(&enemy.intent)
            })
    })
}

fn valid_intent(intent: &Intent) -> bool {
    let targets_valid =
        |targets: &Option<Vec<String>>| targets.as_deref().is_none_or(valid_target_ids);
    match intent {
        Intent::Attack {
            damage: _,
            hits,
            target_ids,
        } => *hits != 0 && targets_valid(target_ids),
        Intent::Defend { target_ids }
        | Intent::Buff { target_ids }
        | Intent::Debuff { target_ids }
        | Intent::Unknown { target_ids } => targets_valid(target_ids),
    }
}

fn valid_choices(choices: Option<&[Choice]>) -> bool {
    choices.is_none_or(|choices| {
        choices.len() <= MAX_ITEMS
            && choices.iter().all(|choice| {
                valid_identity(&choice.choice_id)
                    && valid_text(&choice.label)
                    && valid_identity(&choice.kind)
                    && choice.domain.as_deref().is_none_or(valid_identities)
            })
    })
}

fn valid_action(action: &Action) -> bool {
    let identity = |value: &String| valid_identity(value);
    let optional_identity = |value: &Option<String>| value.as_deref().is_none_or(valid_identity);
    match action {
        Action::StartRun { character_id }
        | Action::SelectCharacter { character_id }
        | Action::SelectMapNode {
            node_id: character_id,
        }
        | Action::RestOption {
            rest_option_id: character_id,
        }
        | Action::ChooseReward {
            reward_id: character_id,
        }
        | Action::ShopPurchase {
            item_id: character_id,
        }
        | Action::ShopRemove {
            card_id: character_id,
        }
        | Action::Smith {
            card_id: character_id,
        }
        | Action::EventChoice {
            choice_id: character_id,
        } => identity(character_id),
        Action::PlayCard { card_id, target_id }
        | Action::UsePotion {
            potion_id: card_id,
            target_id,
        } => identity(card_id) && optional_identity(target_id),
        Action::SelectCard {
            selection_id,
            card_id,
        } => identity(card_id) && optional_identity(selection_id),
        Action::ConfirmSelection { selection_id } | Action::CancelSelection { selection_id } => {
            optional_identity(selection_id)
        }
        Action::EndTurn
        | Action::SkipReward
        | Action::Proceed
        | Action::Rest
        | Action::ConfirmVictory
        | Action::SaveQuit => true,
    }
}

fn valid_target_ids(targets: &[String]) -> bool {
    targets.len() <= MAX_TARGETS && targets.iter().all(|target| valid_identity(target))
}

fn valid_identities(values: &[String]) -> bool {
    values.len() <= MAX_ITEMS && values.iter().all(|value| valid_identity(value))
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TEXT_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

fn valid_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TEXT_BYTES
        && !value
            .chars()
            .any(|character| character.is_control() || (character as u32) == 0x7f)
}

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

fn shape_cards(value: Option<&Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    if value.is_null() {
        return true;
    }
    value.as_array().is_some_and(|items| {
        items.iter().all(|item| {
            exact_object(
                Some(item),
                &[
                    "card_id",
                    "name",
                    "cost",
                    "upgraded",
                    "type",
                    "rarity",
                    "target",
                    "description",
                ],
            )
            .is_some()
        })
    })
}

fn shape_statuses(value: Option<&Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    if value.is_null() {
        return true;
    }
    value.as_array().is_some_and(|items| {
        items
            .iter()
            .all(|item| exact_object(Some(item), &["status_id", "name", "amount"]).is_some())
    })
}

fn shape_relics(value: Option<&Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    if value.is_null() {
        return true;
    }
    value.as_array().is_some_and(|items| {
        items
            .iter()
            .all(|item| exact_object(Some(item), &["relic_id", "name"]).is_some())
    })
}

fn shape_potions(value: Option<&Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    if value.is_null() {
        return true;
    }
    value.as_array().is_some_and(|items| {
        items.iter().all(|item| {
            exact_object(
                Some(item),
                &["potion_id", "name", "slot", "usable", "target_mode"],
            )
            .is_some()
        })
    })
}

fn shape_state(value: Option<&Value>) -> bool {
    let Some(object) = value.and_then(Value::as_object) else {
        return false;
    };
    let Some(state) = object.get("state").and_then(Value::as_str) else {
        return false;
    };
    let fields: &[&str] = match state {
        "setup" => &["state", "characters"],
        "map" => &["state", "current_node_id", "nodes", "edges", "options"],
        "combat" => &["state", "turn_index", "enemies"],
        "reward" | "event" | "rest" | "selection" => &["state", "choices"],
        "shop" => &["state", "items"],
        "victory" => &["state"],
        "defeat" => &["state", "reason"],
        "recovery" => &["state", "code"],
        _ => return false,
    };
    if object.len() != fields.len() || fields.iter().any(|field| !object.contains_key(*field)) {
        return false;
    }
    match state {
        "map" => {
            if !shape_map_nodes(object.get("nodes")) || !shape_map_edges(object.get("edges")) {
                return false;
            }
        }
        "combat" => {
            if !shape_enemies(object.get("enemies")) {
                return false;
            }
        }
        "reward" | "event" | "rest" | "selection" => {
            if !shape_choices(object.get("choices")) {
                return false;
            }
        }
        "shop" => return shape_shop_items(object.get("items")),
        _ => {}
    }
    true
}

fn shape_array(value: Option<&Value>, fields: &[&str]) -> bool {
    let Some(value) = value else {
        return false;
    };
    if value.is_null() {
        return true;
    }
    value.as_array().is_some_and(|items| {
        items
            .iter()
            .all(|item| exact_object(Some(item), fields).is_some())
    })
}

fn shape_map_nodes(value: Option<&Value>) -> bool {
    shape_array(
        value,
        &["node_id", "act", "row", "col", "kind", "reachable"],
    )
}
fn shape_map_edges(value: Option<&Value>) -> bool {
    shape_array(value, &["from", "to"])
}
fn shape_choices(value: Option<&Value>) -> bool {
    shape_array(value, &["choice_id", "label", "kind", "domain"])
}
fn shape_shop_items(value: Option<&Value>) -> bool {
    shape_array(value, &["item_id", "name", "kind", "price"])
}

fn shape_enemies(value: Option<&Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    if value.is_null() {
        return true;
    }
    value.as_array().is_some_and(|items| {
        items.iter().all(|item| {
            let Some(enemy) = exact_object(
                Some(item),
                &[
                    "enemy_id", "name", "hp", "max_hp", "block", "powers", "statuses", "intent",
                ],
            ) else {
                return false;
            };
            shape_statuses(enemy.get("powers"))
                && shape_statuses(enemy.get("statuses"))
                && shape_intent(enemy.get("intent"))
        })
    })
}

fn shape_intent(value: Option<&Value>) -> bool {
    let Some(object) = value.and_then(Value::as_object) else {
        return false;
    };
    let Some(kind) = object.get("kind").and_then(Value::as_str) else {
        return false;
    };
    let fields = if kind == "attack" {
        &["kind", "damage", "hits", "target_ids"][..]
    } else if matches!(kind, "defend" | "buff" | "debuff" | "unknown") {
        &["kind", "target_ids"][..]
    } else {
        return false;
    };
    object.len() == fields.len() && fields.iter().all(|field| object.contains_key(*field))
}

fn shape_legal_actions(value: Option<&Value>) -> bool {
    let Some(items) = value.and_then(Value::as_array) else {
        return false;
    };
    items.iter().all(|item| {
        let Some(object) = exact_object(Some(item), &["action_id", "action"]) else {
            return false;
        };
        shape_action(object.get("action"))
    })
}

fn shape_action(value: Option<&Value>) -> bool {
    let Some(object) = value.and_then(Value::as_object) else {
        return false;
    };
    let Some(kind) = object.get("kind").and_then(Value::as_str) else {
        return false;
    };
    let fields: &[&str] = match kind {
        "start_run" | "select_character" => &["kind", "character_id"],
        "select_map_node" => &["kind", "node_id"],
        "play_card" => &["kind", "card_id", "target_id"],
        "use_potion" => &["kind", "potion_id", "target_id"],
        "end_turn" | "skip_reward" | "proceed" | "rest" | "confirm_victory" | "save_quit" => {
            &["kind"]
        }
        "rest_option" => &["kind", "rest_option_id"],
        "choose_reward" => &["kind", "reward_id"],
        "shop_purchase" => &["kind", "item_id"],
        "shop_remove" | "smith" => &["kind", "card_id"],
        "event_choice" => &["kind", "choice_id"],
        "select_card" => &["kind", "selection_id", "card_id"],
        "confirm_selection" | "cancel_selection" => &["kind", "selection_id"],
        _ => return false,
    };
    object.len() == fields.len() && fields.iter().all(|field| object.contains_key(*field))
}
