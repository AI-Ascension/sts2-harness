// SPDX-License-Identifier: MIT

//! Chooses which legal actions are presented to a provider, and records what was held back.
//!
//! The host-generated catalog is bounded at 256 entries and is authoritative for legality. Handing
//! all of it to a provider is expensive in the shared state-and-question token budget and, for a
//! provider whose confidence is computed from the shape of a probability distribution, actively
//! misleading: twenty near-duplicate options split the mass between strategically identical choices
//! and return a low confidence that reads as "unsure what to do" when it means "unsure which of
//! three identical things to name".
//!
//! This module therefore decides what is *askable*. It never decides what is good: the presented
//! order is the catalog order, and nothing here ranks, scores, or prefers an action.
//!
//! Narrowing may only ever hide an option the host listed, never invent one, so every rule is
//! conservative and every removal is recorded with a reason that names the option it folded into.
//! The fold key is built from the whole action the host emitted rather than from a list of fields
//! this module knows about, so an identity field the vocabulary does not declare still separates two
//! options instead of silently merging them.
//! Two safety properties hold by construction. The first occurrence of any signature is always the
//! presented one, so a turn-ending action in the catalog is always presented and the provider keeps
//! an escape; folding can remove a second copy of it, never the option itself. And a selection that
//! would leave fewer than two options presents the full catalog instead, because one option is not
//! a question. A rule that withholds a whole kind would break the first property, so any such rule
//! has to restore the turn-ending action explicitly; none exists here today.
//!
//! One rule that a first reading suggests is deliberately absent. Withholding an unaffordable card
//! would contradict the host, which lists an action only when it is legal; an affordability filter
//! could therefore only ever remove a play the host said was available. Affordability belongs
//! in the derived-exact facts beside the state, where it informs the provider, not in the
//! option set, where it would overrule the authority.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable schema identity of this selection record.
pub const OPTION_SELECTION_SCHEMA: &str = "ascension.context-control.option-selection.v1";

/// Number of presented options above which a single question is split into two stages.
pub const MAX_PRESENTED_OPTIONS: usize = 24;

/// Declared bound on the legal-action collection, mirroring the model-view vocabulary.
const MAX_CATALOG: usize = 256;

/// Declared bound on the hand collection, mirroring the model-view vocabulary.
const MAX_HAND: usize = 256;

/// How the presented options should be asked about.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionMode {
    /// Exactly one action is legal. No provider call is needed and none should be made.
    Forced,
    /// The presented options fit in one question.
    Single,
    /// The presented options exceed the bound; ask for a kind first, then an action within it.
    TwoStage,
}

/// One option a provider may be asked to choose.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PresentedOption {
    /// The concrete, legal action identifier this option resolves to.
    pub action_id: String,
    /// The action kind, used as the class in a two-stage question.
    pub kind: String,
    /// Other catalog identifiers this option stands for, in catalog order.
    pub folded: Vec<String>,
}

/// Why an option was not presented.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "reason")]
pub enum WithheldReason {
    /// Strategically identical to a presented option under the admitted vocabulary.
    Duplicate {
        /// The presented option this one folded into.
        folded_into: String,
    },
    /// The entry did not carry a usable identifier or action object.
    Malformed,
}

/// One catalog entry that was not presented, with the reason.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WithheldOption {
    /// The catalog identifier that was held back, when it had one.
    pub action_id: String,
    /// Why it was held back.
    #[serde(flatten)]
    pub reason: WithheldReason,
}

/// The option set a provider is asked about, and everything that was held back.
///
/// `presented` and `withheld` partition the catalog: every entry appears in exactly one of them.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionSelection {
    /// How to ask about the presented options.
    pub mode: SelectionMode,
    /// The options to present, in catalog order.
    pub presented: Vec<PresentedOption>,
    /// The entries held back, each naming its reason.
    pub withheld: Vec<WithheldOption>,
}

impl OptionSelection {
    /// Selects the options to present from an admitted observation's legal-action catalog.
    ///
    /// `bound` is the number of options above which the selection asks for a two-stage question.
    #[must_use]
    pub fn from_observation(observation: &Value, bound: usize) -> Self {
        let catalog = observation
            .get("legal_actions")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let hand = observation
            .get("player")
            .and_then(|player| player.get("hand"))
            .and_then(Value::as_array);
        let entries = read_entries(catalog, hand);
        let full: Vec<PresentedOption> = entries
            .iter()
            .map(|entry| PresentedOption {
                action_id: entry.action_id.to_owned(),
                kind: entry.kind.to_owned(),
                folded: Vec::new(),
            })
            .collect();
        let malformed: Vec<WithheldOption> = catalog
            .iter()
            .take(MAX_CATALOG)
            .filter(|item| read_entry(item, hand).is_none())
            .map(|item| WithheldOption {
                action_id: item
                    .get("action_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                reason: WithheldReason::Malformed,
            })
            .collect();

        if full.len() == 1 {
            return Self {
                mode: SelectionMode::Forced,
                presented: full,
                withheld: malformed,
            };
        }
        let (presented, mut withheld) = collapse(&entries);
        // Fewer than two askable options is not a question; present the catalog unchanged.
        let (presented, withheld) = if presented.len() < 2 {
            (full, malformed)
        } else {
            withheld.extend(malformed.iter().cloned());
            (presented, withheld)
        };
        let mode = if presented.len() > bound {
            SelectionMode::TwoStage
        } else {
            SelectionMode::Single
        };
        Self {
            mode,
            presented,
            withheld,
        }
    }

    /// The distinct action kinds of the presented options, in first-seen order.
    #[must_use]
    pub fn classes(&self) -> Vec<String> {
        let mut classes: Vec<String> = Vec::new();
        for option in &self.presented {
            if !classes.iter().any(|kind| kind == &option.kind) {
                classes.push(option.kind.clone());
            }
        }
        classes
    }

    /// The presented options of one kind, in catalog order.
    ///
    /// The second stage of a two-stage ask is restricted to exactly these options once the first
    /// stage has named the kind, so the action question never re-offers an option outside the chosen
    /// class and never offers one the host did not list.
    #[must_use]
    pub fn options_of_kind(&self, kind: &str) -> Vec<&PresentedOption> {
        self.presented
            .iter()
            .filter(|option| option.kind == kind)
            .collect()
    }
}

/// One readable catalog entry.
struct Entry<'a> {
    action_id: &'a str,
    kind: &'a str,
    signature: String,
}

/// Reads every usable catalog entry, in catalog order.
fn read_entries<'a>(catalog: &'a [Value], hand: Option<&Vec<Value>>) -> Vec<Entry<'a>> {
    catalog
        .iter()
        .take(MAX_CATALOG)
        .filter_map(|item| read_entry(item, hand))
        .collect()
}

/// Reads one catalog entry, refusing an entry without an identifier or an action object.
///
/// The signature is built from the **whole** action object, so it is injective on whatever the host
/// emits — including fields this repository's model-view vocabulary does not declare, such as
/// `potion_id`, `rest_option_id` and `selection_id`. An earlier version keyed on a fixed list of
/// seven fields and silently folded distinct actions that differed only outside it; two different
/// potions aimed at one enemy, or two different rest options, were indistinguishable.
///
/// Exactly one substitution is made, and it is the only thing that folds anything: a `card_id` that
/// resolves to a card in hand is replaced by that card's identity — name, cost, upgraded — so two
/// copies of one card aimed at the same target share a signature. A `card_id` that does not resolve
/// is kept verbatim, so an unresolvable card never folds with anything.
fn read_entry<'a>(item: &'a Value, hand: Option<&Vec<Value>>) -> Option<Entry<'a>> {
    let action_id = item.get("action_id").and_then(Value::as_str)?;
    let action = item.get("action")?.as_object()?;
    let kind = action.get("kind").and_then(Value::as_str)?;
    let mut signature = serde_json::Map::new();
    for (name, value) in action {
        let folded = (name == "card_id")
            .then(|| value.as_str().and_then(|card| card_identity(card, hand)))
            .flatten();
        signature.insert(
            name.clone(),
            folded.map_or_else(|| value.clone(), Value::String),
        );
    }
    Some(Entry {
        action_id,
        kind,
        // Serialized from a map, so field order is canonical rather than emission order.
        signature: Value::Object(signature).to_string(),
    })
}

/// The identity of a card in hand: what makes two copies of it the same question.
///
/// `None` when the hand is absent or the identifier does not resolve, which keeps the instance
/// identifier in the signature and prevents a fold that cannot be justified.
fn card_identity(card_id: &str, hand: Option<&Vec<Value>>) -> Option<String> {
    let card = hand?
        .iter()
        .take(MAX_HAND)
        .find(|card| card.get("card_id").and_then(Value::as_str) == Some(card_id))?;
    let name = card.get("name").and_then(Value::as_str)?;
    let cost = card.get("cost").and_then(Value::as_i64)?;
    let upgraded = card.get("upgraded").and_then(Value::as_bool)?;
    Some(format!("card-identity:{name}:{cost}:{upgraded}"))
}

/// Folds entries whose signatures are equal into their first occurrence.
///
/// Two entries fold only when every field the host emitted matches, after a card in hand has been
/// replaced by its identity. Two copies of one card aimed at the same target therefore fold; two
/// different cards, potions, rest options or selections never do. The first occurrence in catalog
/// order is the representative, so the result is stable for a fixed input.
fn collapse(entries: &[Entry<'_>]) -> (Vec<PresentedOption>, Vec<WithheldOption>) {
    let mut presented: Vec<PresentedOption> = Vec::new();
    let mut signatures: Vec<&str> = Vec::new();
    let mut withheld = Vec::new();
    for entry in entries {
        let seen = signatures
            .iter()
            .position(|signature| *signature == entry.signature.as_str());
        match seen.and_then(|index| presented.get_mut(index)) {
            Some(option) => {
                option.folded.push(entry.action_id.to_owned());
                withheld.push(WithheldOption {
                    action_id: entry.action_id.to_owned(),
                    reason: WithheldReason::Duplicate {
                        folded_into: option.action_id.clone(),
                    },
                });
            }
            None => {
                signatures.push(entry.signature.as_str());
                presented.push(PresentedOption {
                    action_id: entry.action_id.to_owned(),
                    kind: entry.kind.to_owned(),
                    folded: Vec::new(),
                });
            }
        }
    }
    (presented, withheld)
}

#[cfg(test)]
#[path = "option_selection_tests.rs"]
mod tests;
