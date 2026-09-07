// SPDX-License-Identifier: MIT

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
#[serde(deny_unknown_fields, tag = "kind")]
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
    #[serde(rename = "composite")]
    Composite {
        intents: Vec<IntentComponent>,
        target_ids: Option<Vec<String>>,
    },
}
