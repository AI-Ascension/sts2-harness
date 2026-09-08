// SPDX-License-Identifier: MIT

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
        Intent::Composite {
            intents,
            target_ids,
        } => {
            intents.len() >= 2
                && intents.len() <= MAX_TARGETS
                && intents.iter().all(valid_intent_component)
                && targets_valid(target_ids)
        }
    }
}

fn valid_intent_component(intent: &IntentComponent) -> bool {
    let targets_valid =
        |targets: &Option<Vec<String>>| targets.as_deref().is_none_or(valid_target_ids);
    match intent {
        IntentComponent::Attack {
            hits, target_ids, ..
        } => *hits != 0 && targets_valid(target_ids),
        IntentComponent::Defend { target_ids }
        | IntentComponent::Buff { target_ids }
        | IntentComponent::Debuff { target_ids }
        | IntentComponent::Unknown { target_ids } => targets_valid(target_ids),
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
